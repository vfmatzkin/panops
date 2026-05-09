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

#[cfg(test)]
mod storage_fake_tests {
    use super::InMemoryStorage;
    use crate::conformance::storage::run_suite;

    #[test]
    fn in_memory_storage_passes_conformance() {
        run_suite(&InMemoryStorage::new());
    }
}
