use std::path::Path;

use crate::asr::{AsrError, AsrProvider};
use crate::diar::{DiarError, Diarizer, SpeakerTurn};
use crate::{Segment, Transcript};

/// A degenerate `AsrProvider` that reads `<audio>.transcript.txt` from disk
/// and returns a single `Segment` covering the entire audio. Language is
/// inferred from the filename prefix. Used by `panops-core`'s own test
/// crate to validate the conformance harness end-to-end without ML.
pub struct TranscriptFileFake;

impl AsrProvider for TranscriptFileFake {
    fn transcribe_full(
        &self,
        audio_path: &Path,
        _language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        if !audio_path.exists() {
            return Err(AsrError::AudioNotFound(audio_path.to_path_buf()));
        }
        let transcript_path = audio_path.with_extension("transcript.txt");
        let text = std::fs::read_to_string(&transcript_path)
            .map_err(|e| {
                AsrError::InvalidAudio(format!("failed reading sidecar {transcript_path:?}: {e}"))
            })?
            .trim()
            .to_string();

        let reader = hound::WavReader::open(audio_path)
            .map_err(|e| AsrError::InvalidAudio(e.to_string()))?;
        let spec = reader.spec();
        #[allow(clippy::cast_precision_loss)]
        let total_samples = reader.duration() as f64;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let audio_duration_ms = (total_samples * 1000.0 / f64::from(spec.sample_rate)) as u64;

        let language = infer_language(audio_path);

        Ok(Transcript {
            schema_version: Transcript::SCHEMA_VERSION,
            model: "transcript-file-fake".to_string(),
            audio_path: audio_path.to_path_buf(),
            audio_duration_ms,
            diarized: false,
            segments: vec![Segment {
                start_ms: 0,
                end_ms: audio_duration_ms,
                text,
                language_detected: language,
                confidence: 1.0,
                is_partial: false,
                speaker_id: None,
            }],
        })
    }

    fn is_fake(&self) -> bool {
        true
    }
}

fn infer_language(audio_path: &Path) -> Option<String> {
    let stem = audio_path.file_stem()?.to_str()?;
    if stem.starts_with("en_") || stem.starts_with("multi_speaker_") {
        Some("en".to_string())
    } else if stem.starts_with("es_") {
        Some("es".to_string())
    } else if stem.starts_with("mixed_") {
        Some("en".to_string())
    } else {
        None
    }
}

/// A `Diarizer` fake that reads `<audio>.turns.json` and returns it verbatim.
/// Used to validate the conformance harness without ML.
pub struct KnownTurnsFake;

impl Diarizer for KnownTurnsFake {
    fn diarize(&self, audio_path: &Path) -> Result<Vec<SpeakerTurn>, DiarError> {
        if !audio_path.exists() {
            return Err(DiarError::AudioNotFound(audio_path.to_path_buf()));
        }
        let turns_path = audio_path.with_extension("turns.json");
        let body = std::fs::read_to_string(&turns_path)
            .map_err(|e| DiarError::InvalidAudio(format!("read sidecar {turns_path:?}: {e}")))?;
        let turns: Vec<SpeakerTurn> = serde_json::from_str(&body)
            .map_err(|e| DiarError::Diarization(format!("parse {turns_path:?}: {e}")))?;
        Ok(turns)
    }

    fn is_fake(&self) -> bool {
        true
    }
}

use std::collections::HashMap;
use std::sync::Mutex;

use crate::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse, prompt_fingerprint};

/// Deterministic `LlmProvider` fake. Tests register `(system, user) ->
/// response` pairs via `with_response_for`, or `(system, user) -> error`
/// pairs via `with_error_for`. Unmatched prompts panic loudly so prompt
/// drift is caught immediately.
#[derive(Default)]
pub struct MockLlm {
    table: Mutex<HashMap<String, Result<LlmResponse, String>>>,
}

impl MockLlm {
    pub fn with_response_for(
        self,
        system: Option<&str>,
        user: &str,
        response: LlmResponse,
    ) -> Self {
        let key = prompt_fingerprint(system, user);
        self.table.lock().unwrap().insert(key, Ok(response));
        self
    }

    pub fn with_error_for(self, system: Option<&str>, user: &str, message: &str) -> Self {
        let key = prompt_fingerprint(system, user);
        self.table
            .lock()
            .unwrap()
            .insert(key, Err(message.to_string()));
        self
    }
}

impl LlmProvider for MockLlm {
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let key = prompt_fingerprint(req.system.as_deref(), &req.user);
        let map = self.table.lock().unwrap();
        match map.get(&key) {
            Some(Ok(r)) => Ok(r.clone()),
            Some(Err(msg)) => Err(LlmError::Provider(msg.clone())),
            None => panic!(
                "MockLlm: no canned response for prompt fingerprint {key}\nsystem={:?}\nuser={:?}",
                req.system, req.user
            ),
        }
    }
}

/// Minimal `NotesExporter` for the conformance harness. Writes a single
/// `notes.txt` file containing the rendered title and section count under
/// `dest`. Refuses (with `ExportError::InvalidDest`) when `dest` exists but
/// is a file rather than a directory — this is the universal contract every
/// exporter must hold to avoid silent data loss.
pub struct FakeNotesExporter;

impl crate::exporter::NotesExporter for FakeNotesExporter {
    fn export(
        &self,
        notes: &crate::notes::ir::StructuredNotes,
        dest: &std::path::Path,
    ) -> Result<crate::exporter::ExportArtifact, crate::exporter::ExportError> {
        if dest.exists() && !dest.is_dir() {
            return Err(crate::exporter::ExportError::InvalidDest(format!(
                "{dest:?} exists but is not a directory"
            )));
        }
        if !dest.exists() {
            std::fs::create_dir_all(dest)?;
        }
        let primary_file = dest.join("notes.txt");
        let body = format!(
            "title: {}\nsections: {}\n",
            notes.frontmatter.title,
            notes.sections.len()
        );
        std::fs::write(&primary_file, body)?;
        Ok(crate::exporter::ExportArtifact {
            primary_file,
            assets: vec![],
        })
    }
}

// === InMemoryStorage ============================================

use chrono::Utc;

use crate::storage::{
    Meeting, MeetingDraft, MeetingSummary, Note, NoteDraft, Storage, StorageError,
};

/// In-process `Storage` fake. Used by `panops-core`'s own tests and
/// by `panops-engine`'s IPC integration tests where opening a real
/// SQLite DB on every run would be unnecessary friction.
pub struct InMemoryStorage {
    inner: Mutex<InMemoryInner>,
}

struct InMemoryInner {
    meetings: HashMap<String, Meeting>,
    notes: HashMap<String, Note>,
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(InMemoryInner {
                meetings: HashMap::new(),
                notes: HashMap::new(),
            }),
        }
    }
}

impl Storage for InMemoryStorage {
    fn create_meeting(&self, d: MeetingDraft) -> Result<Meeting, StorageError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.meetings.contains_key(&d.id) {
            return Err(StorageError::AlreadyExists {
                id: d.id,
                kind: "meeting",
            });
        }
        // Mirror the schema's `dir_path TEXT NOT NULL UNIQUE` so the
        // fake and real adapter behave identically. Without this,
        // tests that pass against the fake could break against the
        // real adapter on a dir_path collision.
        if inner.meetings.values().any(|m| m.dir_path == d.dir_path) {
            return Err(StorageError::UniqueConflict {
                kind: "meeting",
                field: "dir_path",
                value: d.dir_path,
            });
        }
        let m = Meeting {
            id: d.id.clone(),
            title: d.title,
            started_at: d.started_at,
            ended_at: None,
            duration_ms: None,
            language: d.language,
            dir_path: d.dir_path,
        };
        inner.meetings.insert(d.id, m.clone());
        Ok(m)
    }

    fn create_meeting_with_note(
        &self,
        meeting: MeetingDraft,
        note: NoteDraft,
        ended_at: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<(Meeting, Note), StorageError> {
        // Validate everything up front under a single lock so partial
        // commits aren't visible. Real adapter (RusqliteStorage) does
        // this with `BEGIN`/`COMMIT`; we fake atomicity here by holding
        // the lock for the whole sequence and only mutating after all
        // checks pass.
        let mut inner = self.inner.lock().unwrap();
        if inner.meetings.contains_key(&meeting.id) {
            return Err(StorageError::AlreadyExists {
                id: meeting.id,
                kind: "meeting",
            });
        }
        if inner
            .meetings
            .values()
            .any(|m| m.dir_path == meeting.dir_path)
        {
            return Err(StorageError::UniqueConflict {
                kind: "meeting",
                field: "dir_path",
                value: meeting.dir_path,
            });
        }
        if note.meeting_id != meeting.id {
            return Err(StorageError::Sql {
                message: "create_meeting_with_note: note.meeting_id must match meeting.id".into(),
            });
        }
        if inner.notes.contains_key(&note.id) {
            return Err(StorageError::AlreadyExists {
                id: note.id,
                kind: "note",
            });
        }
        // All checks passed — commit both rows.
        let m = Meeting {
            id: meeting.id.clone(),
            title: meeting.title,
            started_at: meeting.started_at,
            ended_at: ended_at.map(str::to_owned),
            duration_ms,
            language: meeting.language,
            dir_path: meeting.dir_path,
        };
        let n = Note {
            id: note.id.clone(),
            meeting_id: note.meeting_id,
            dialect: note.dialect,
            content_md: note.content_md,
            primary_path: note.primary_path,
            created_at: Utc::now().to_rfc3339(),
        };
        inner.meetings.insert(meeting.id, m.clone());
        inner.notes.insert(note.id, n.clone());
        Ok((m, n))
    }

    fn get_meeting(&self, id: &str) -> Result<Meeting, StorageError> {
        let inner = self.inner.lock().unwrap();
        inner
            .meetings
            .get(id)
            .cloned()
            .ok_or_else(|| StorageError::NotFound {
                id: id.into(),
                kind: "meeting",
            })
    }

    fn list_meetings(&self) -> Result<Vec<MeetingSummary>, StorageError> {
        let inner = self.inner.lock().unwrap();
        let mut rows: Vec<MeetingSummary> = inner
            .meetings
            .values()
            .map(|m| MeetingSummary {
                id: m.id.clone(),
                title: m.title.clone(),
                started_at: m.started_at.clone(),
                duration_ms: m.duration_ms.unwrap_or(0),
            })
            .collect();
        rows.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(rows)
    }

    fn update_meeting_ended(
        &self,
        id: &str,
        ended_at: &str,
        duration_ms: u64,
    ) -> Result<Meeting, StorageError> {
        let mut inner = self.inner.lock().unwrap();
        let m = inner
            .meetings
            .get_mut(id)
            .ok_or_else(|| StorageError::NotFound {
                id: id.into(),
                kind: "meeting",
            })?;
        m.ended_at = Some(ended_at.into());
        m.duration_ms = Some(duration_ms);
        Ok(m.clone())
    }

    fn update_meeting_language(&self, id: &str, language: &str) -> Result<Meeting, StorageError> {
        let mut inner = self.inner.lock().unwrap();
        let m = inner
            .meetings
            .get_mut(id)
            .ok_or_else(|| StorageError::NotFound {
                id: id.into(),
                kind: "meeting",
            })?;
        m.language = language.into();
        Ok(m.clone())
    }

    fn delete_meeting(&self, id: &str) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.meetings.remove(id).is_none() {
            return Err(StorageError::NotFound {
                id: id.into(),
                kind: "meeting",
            });
        }
        // FK cascade simulation.
        inner.notes.retain(|_, n| n.meeting_id != id);
        Ok(())
    }

    fn create_note(&self, d: NoteDraft) -> Result<Note, StorageError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.meetings.contains_key(&d.meeting_id) {
            return Err(StorageError::NotFound {
                id: d.meeting_id,
                kind: "meeting",
            });
        }
        if inner.notes.contains_key(&d.id) {
            return Err(StorageError::AlreadyExists {
                id: d.id,
                kind: "note",
            });
        }
        let n = Note {
            id: d.id.clone(),
            meeting_id: d.meeting_id,
            dialect: d.dialect,
            content_md: d.content_md,
            primary_path: d.primary_path,
            created_at: Utc::now().to_rfc3339(),
        };
        inner.notes.insert(d.id, n.clone());
        Ok(n)
    }

    fn list_notes_for_meeting(&self, meeting_id: &str) -> Result<Vec<Note>, StorageError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .notes
            .values()
            .filter(|n| n.meeting_id == meeting_id)
            .cloned()
            .collect())
    }
}

// === KnownRegionsFake ============================================

use crate::vad::{SpeechRegion, Vad, VadError};

/// Deterministic `Vad` fake that detects regions wherever the
/// **absolute** sample value exceeds a small threshold. Used by
/// `panops-core`'s own conformance test and by `panops-engine`'s
/// integration tests where loading a real VAD model would be
/// unnecessary friction. Threshold is `1e-3` (sine waves of
/// amplitude 0.5 trip it; pure-silent samples at `0.0` don't).
pub struct KnownRegionsFake {
    /// Frame size in milliseconds. Samples are bucketed into frames
    /// of this size and a frame is "speech" if its peak abs sample
    /// is above `threshold`. Defaults to 100 ms.
    pub frame_ms: u64,
    pub threshold: f32,
}

impl Default for KnownRegionsFake {
    fn default() -> Self {
        Self {
            frame_ms: 100,
            threshold: 1e-3,
        }
    }
}

impl KnownRegionsFake {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Vad for KnownRegionsFake {
    fn detect_speech(
        &self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<SpeechRegion>, VadError> {
        if sample_rate != 16_000 {
            return Err(VadError::InvalidAudio(format!(
                "expected 16 kHz, got {sample_rate} Hz"
            )));
        }
        let frame_size_samples = ((sample_rate as u64 * self.frame_ms) / 1000) as usize;
        if frame_size_samples == 0 {
            return Err(VadError::InvalidAudio(
                "frame_ms produces zero samples per frame".into(),
            ));
        }

        let mut regions: Vec<SpeechRegion> = Vec::new();
        let mut current_start: Option<u64> = None;
        let total_chunks = samples.chunks(frame_size_samples).count();
        for (i, frame) in samples.chunks(frame_size_samples).enumerate() {
            let peak = frame.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
            let frame_start_ms = (i as u64) * self.frame_ms;
            let frame_end_ms =
                frame_start_ms + (frame.len() as u64 * 1000) / u64::from(sample_rate);

            if peak >= self.threshold {
                if current_start.is_none() {
                    current_start = Some(frame_start_ms);
                }
            } else if let Some(start) = current_start.take() {
                regions.push(SpeechRegion {
                    start_ms: start,
                    end_ms: frame_end_ms - (frame.len() as u64 * 1000) / u64::from(sample_rate),
                });
            }
            // If we're on the last chunk and still in-speech, close the region.
            if i + 1 == total_chunks {
                if let Some(start) = current_start.take() {
                    regions.push(SpeechRegion {
                        start_ms: start,
                        end_ms: frame_end_ms,
                    });
                }
            }
        }
        Ok(regions)
    }

    fn is_fake(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod vad_fake_tests {
    use super::KnownRegionsFake;
    use crate::conformance::vad::run_suite;

    #[test]
    fn known_regions_fake_passes_conformance() {
        run_suite(&KnownRegionsFake::new());
    }
}

#[cfg(test)]
mod storage_fake_tests {
    use super::InMemoryStorage;
    use crate::conformance::storage::run_suite;

    #[test]
    fn in_memory_storage_passes_conformance() {
        run_suite(&InMemoryStorage::new());
    }
}
