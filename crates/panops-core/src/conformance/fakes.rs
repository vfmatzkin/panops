use std::path::Path;
use std::sync::Mutex;

use crate::asr::{AsrError, AsrProvider};
use crate::diar::{DiarError, Diarizer, SpeakerTurn};
use crate::vad::{SpeechRegion, Vad, VadError};
use crate::{Segment, Transcript};

/// A degenerate `AsrProvider` fake. Constructed with a canned
/// `Transcript` (typically built via `read_canned_sidecar`); each
/// `transcribe(samples, ...)` call returns that canned transcript
/// after recomputing `audio_duration_ms` from the samples.
///
/// The `<audio>.transcript.txt` sidecar pattern from slice 02 lives
/// in `read_canned_sidecar` below — kept as a test helper but no
/// longer the AsrProvider impl itself (the new samples-based ASR
/// trait doesn't see file paths).
pub struct TranscriptFileFake {
    canned: Transcript,
}

impl Default for TranscriptFileFake {
    fn default() -> Self {
        Self {
            canned: Transcript {
                schema_version: Transcript::SCHEMA_VERSION,
                model: "transcript-file-fake".to_string(),
                audio_path: std::path::PathBuf::new(),
                audio_duration_ms: 0,
                diarized: false,
                segments: vec![Segment {
                    start_ms: 0,
                    end_ms: 0,
                    text: String::new(),
                    language_detected: Some("en".to_string()),
                    confidence: 1.0,
                    is_partial: false,
                    speaker_id: None,
                }],
            },
        }
    }
}

impl TranscriptFileFake {
    /// Build a fake from a canned transcript. The `audio_path` field
    /// is overwritten by the caller (the pipeline) when needed.
    pub fn with_canned(canned: Transcript) -> Self {
        Self { canned }
    }

    /// Convenience constructor: returns a fake whose canned transcript
    /// is a single segment containing `text` for the given language.
    pub fn from_text(text: &str, language: Option<&str>) -> Self {
        Self {
            canned: Transcript {
                schema_version: Transcript::SCHEMA_VERSION,
                model: "transcript-file-fake".to_string(),
                audio_path: std::path::PathBuf::new(),
                audio_duration_ms: 0,
                diarized: false,
                segments: vec![Segment {
                    start_ms: 0,
                    end_ms: 0,
                    text: text.to_string(),
                    language_detected: language.map(str::to_string),
                    confidence: 1.0,
                    is_partial: false,
                    speaker_id: None,
                }],
            },
        }
    }
}

impl AsrProvider for TranscriptFileFake {
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        _language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        if sample_rate != 16_000 {
            return Err(AsrError::InvalidAudio(format!(
                "expected 16 kHz, got {sample_rate} Hz"
            )));
        }
        let audio_duration_ms = (samples.len() as u64 * 1000) / u64::from(sample_rate);
        let mut t = self.canned.clone();
        t.audio_duration_ms = audio_duration_ms;
        if let Some(seg) = t.segments.first_mut() {
            seg.start_ms = 0;
            seg.end_ms = audio_duration_ms;
        }
        Ok(t)
    }

    fn is_fake(&self) -> bool {
        true
    }
}

/// Build a canned `Transcript` from the slice-02 sidecar pattern:
/// reads `<audio>.transcript.txt` for the text and infers the
/// language from the audio file stem (`en_*` / `es_*` / `mixed_*` /
/// `multi_speaker_*`). Used by the ASR conformance harness which
/// needs to wire a fixture-aware fake into the new samples-based
/// trait.
pub fn read_canned_sidecar(audio_path: &Path) -> Transcript {
    let transcript_path = audio_path.with_extension("transcript.txt");
    let text = std::fs::read_to_string(&transcript_path)
        .unwrap_or_else(|e| panic!("read sidecar {transcript_path:?}: {e}"))
        .trim()
        .to_string();
    let language = infer_language(audio_path);
    Transcript {
        schema_version: Transcript::SCHEMA_VERSION,
        model: "transcript-file-fake".to_string(),
        audio_path: audio_path.to_path_buf(),
        audio_duration_ms: 0,
        diarized: false,
        segments: vec![Segment {
            start_ms: 0,
            end_ms: 0,
            text,
            language_detected: language,
            confidence: 1.0,
            is_partial: false,
            speaker_id: None,
        }],
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

/// Test fake for `recursive_asr`. Returns canned `Transcript` responses
/// indexed by call count. Once the response list is exhausted, the last
/// response is returned for any further calls (mirrors the "always
/// low-confidence" MAX_DEPTH-cap test case).
///
/// Tests construct it via `LowConfidenceAsr::with_responses(vec![...])`.
/// Optionally chain `.with_error_after(n)` to inject an
/// `AsrError::Transcription` on call index `n` and after — used to
/// exercise the error-propagation path through a recursive frame.
pub struct LowConfidenceAsr {
    responses: Vec<Transcript>,
    error_after: Option<usize>,
    call_count: Mutex<usize>,
}

impl LowConfidenceAsr {
    pub fn with_responses(responses: Vec<Transcript>) -> Self {
        assert!(
            !responses.is_empty(),
            "LowConfidenceAsr needs at least one canned response"
        );
        Self {
            responses,
            error_after: None,
            call_count: Mutex::new(0),
        }
    }

    /// Inject an `AsrError::Transcription` on call index `n` and every
    /// call after. Used by the recursive-frame error-propagation test.
    pub fn with_error_after(mut self, n: usize) -> Self {
        self.error_after = Some(n);
        self
    }

    pub fn call_count(&self) -> usize {
        *self.call_count.lock().expect("call_count mutex poisoned")
    }
}

impl AsrProvider for LowConfidenceAsr {
    fn transcribe(
        &self,
        _samples: &[f32],
        _sample_rate: u32,
        _language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        let mut guard = self.call_count.lock().expect("call_count mutex poisoned");
        let idx = *guard;
        *guard += 1;
        if self.error_after.is_some_and(|n| idx >= n) {
            return Err(AsrError::Transcription(format!(
                "LowConfidenceAsr injected error at call {idx}"
            )));
        }
        let resp_idx = idx.min(self.responses.len() - 1);
        Ok(self.responses[resp_idx].clone())
    }

    fn is_fake(&self) -> bool {
        true
    }
}

use std::collections::HashMap;

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

/// A `LlmProvider` fake whose backend always fails. Drives the error-path
/// conformance checks (`conformance::llm::run_error_suite` /
/// `assert_empty_response`): a live LLM can't be made to fail on command, so
/// the error contract is validated through this reference failing impl.
///
/// `FailingLlm::provider(msg)` models a backend transport/provider failure;
/// `FailingLlm::empty()` models a backend that returned a blank completion,
/// which adapters must surface as `LlmError::EmptyResponse` (see
/// `panops-portable`'s `GenaiLlm`, which maps an empty `first_text()` the same
/// way). The prompt is ignored.
pub struct FailingLlm {
    mode: FailMode,
}

enum FailMode {
    Provider(String),
    Empty,
}

impl FailingLlm {
    /// Models a backend transport/provider failure.
    pub fn provider(message: &str) -> Self {
        Self {
            mode: FailMode::Provider(message.to_string()),
        }
    }

    /// Models a backend that returned an empty/blank completion.
    pub fn empty() -> Self {
        Self {
            mode: FailMode::Empty,
        }
    }
}

impl LlmProvider for FailingLlm {
    fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        match &self.mode {
            FailMode::Provider(message) => Err(LlmError::Provider(message.clone())),
            FailMode::Empty => Err(LlmError::EmptyResponse),
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

/// Deterministic `Vad` fake that detects regions wherever the
/// **absolute** sample value exceeds a small threshold. Used by
/// `panops-core`'s own conformance test and by `panops-engine`'s
/// integration tests where loading a real VAD model would be
/// unnecessary friction. Threshold is `1e-3` (any sample with
/// `|s| >= threshold` triggers detection; the `1e-3` default rejects
/// pure-silent (`0.0`) samples).
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

impl KnownRegionsFake {}

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
        for (i, frame) in samples.chunks(frame_size_samples).enumerate() {
            let peak = frame.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
            let frame_start_ms = (i as u64) * self.frame_ms;

            if peak >= self.threshold {
                if current_start.is_none() {
                    current_start = Some(frame_start_ms);
                }
            } else if let Some(start) = current_start.take() {
                regions.push(SpeechRegion {
                    start_ms: start,
                    end_ms: frame_start_ms,
                });
            }
        }
        // Close any trailing open region after the loop.
        if let Some(start) = current_start.take() {
            let trailing_end_ms = (samples.len() as u64 * 1000) / u64::from(sample_rate);
            regions.push(SpeechRegion {
                start_ms: start,
                end_ms: trailing_end_ms,
            });
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
        run_suite(&KnownRegionsFake::default());
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

// === FakeCapture ============================================

use std::f32::consts::PI;
use std::path::PathBuf;

use crate::capture::{
    AudioSources, Capture, CaptureConfig, CaptureError, CaptureResult, CaptureSession,
};

/// A fake `Capture` adapter for testing. Produces:
/// - A synthetic 440 Hz sine wave PCM written to a WAV file via `hound`.
/// - Pre-generated screenshot fixtures from `tests/fixtures/screenshots/`.
///
/// Thread-safe: sessions are tracked in an internal `Mutex<HashMap>`.
pub struct FakeCapture {
    sessions: Mutex<std::collections::HashMap<String, (PathBuf, u64, AudioSources)>>,
}

impl Default for FakeCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCapture {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl Capture for FakeCapture {
    fn start_capture(
        &self,
        meeting_id: &str,
        meeting_dir: &std::path::Path,
        config: &CaptureConfig,
    ) -> Result<CaptureSession, CaptureError> {
        let started_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| CaptureError::Capture(e.to_string()))?
            .as_millis() as u64;

        // Store session info for stop_capture.
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| CaptureError::Capture("mutex poisoned".into()))?;
        sessions.insert(
            meeting_id.to_string(),
            (
                meeting_dir.to_path_buf(),
                started_at_ms,
                config.audio_sources,
            ),
        );

        Ok(CaptureSession {
            meeting_id: meeting_id.to_string(),
            started_at_ms,
        })
    }

    fn stop_capture(&self, session: &CaptureSession) -> Result<CaptureResult, CaptureError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| CaptureError::Capture("mutex poisoned".into()))?;
        let (meeting_dir, started_at_ms, audio_sources) = sessions
            .remove(&session.meeting_id)
            .ok_or_else(|| CaptureError::SessionNotFound(session.meeting_id.clone()))?;

        // Compute duration (at least 1s for valid WAV).
        let ended_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| CaptureError::Capture(e.to_string()))?
            .as_millis() as u64;
        let duration_ms = ended_at_ms.saturating_sub(started_at_ms).max(1000);

        // Write one synthetic 16 kHz mono WAV per requested source — never a
        // mixed track (slice 11, Decision §2). Each field is `Some` only when
        // its source was requested via `audio_sources`.
        let want_system = !matches!(audio_sources, AudioSources::MicOnly);
        let want_mic = !matches!(audio_sources, AudioSources::SystemOnly);
        let system_audio_path = if want_system {
            Some(write_sine_wav(
                &meeting_dir.join("system.wav"),
                duration_ms,
            )?)
        } else {
            None
        };
        let mic_audio_path = if want_mic {
            Some(write_sine_wav(&meeting_dir.join("mic.wav"), duration_ms)?)
        } else {
            None
        };

        // Write self-contained synthetic screenshot placeholders. The fake
        // must NOT depend on tests/fixtures being an ancestor of meeting_dir
        // (the engine's per-meeting dir lives outside the workspace, so the
        // old ancestor-walk failed there). Emit a minimal embedded JPEG
        // (SOI + JFIF APP0 + EOI) instead — enough for the contract, which
        // only requires the screenshot paths to exist.
        let screenshots_dir = meeting_dir.join("screenshots");
        std::fs::create_dir_all(&screenshots_dir).map_err(CaptureError::Io)?;

        const FAKE_JPEG: &[u8] = &[
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00,
            0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
        ];
        let mut screenshot_paths: Vec<PathBuf> = Vec::new();
        for i in 1..=3 {
            let dest_path = screenshots_dir.join(format!("{:03}.jpg", i));
            std::fs::write(&dest_path, FAKE_JPEG).map_err(CaptureError::Io)?;
            screenshot_paths.push(dest_path);
        }

        Ok(CaptureResult {
            system_audio_path,
            mic_audio_path,
            screenshot_paths,
            duration_ms,
        })
    }

    fn is_fake(&self) -> bool {
        true
    }
}

/// Write a synthetic 440 Hz sine WAV (16 kHz mono) of `duration_ms` and
/// return its path. Used by `FakeCapture` for each requested audio track.
fn write_sine_wav(path: &Path, duration_ms: u64) -> Result<PathBuf, CaptureError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| CaptureError::Capture(e.to_string()))?;
    let num_samples = (16_000 * duration_ms / 1000) as usize;
    let freq = 440.0_f32;
    for i in 0..num_samples {
        let t = i as f32 / 16_000.0_f32;
        let sample = (2.0_f32 * PI * freq * t).sin();
        let amplitude = (sample * 0.3 * i16::MAX as f32) as i16;
        writer
            .write_sample(amplitude)
            .map_err(|e| CaptureError::Capture(e.to_string()))?;
    }
    writer
        .finalize()
        .map_err(|e| CaptureError::Capture(e.to_string()))?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod capture_fake_tests {
    use super::FakeCapture;
    use crate::conformance::capture::run_suite;
    use std::path::PathBuf;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|p| p.join("tests/fixtures/screenshots").is_dir())
            .expect("workspace root with fixtures")
            .join("tests/fixtures")
    }

    #[test]
    fn fake_capture_passes_conformance() {
        let adapter = FakeCapture::new();
        run_suite(&adapter, &fixtures_dir());
    }
}

// === InMemoryMeetingStore ============================================

use crate::meeting_store::{
    MeetingStore, MeetingStoreError, ScreenshotDraft, ScreenshotRow, SegmentDraft, SegmentRow,
    SpeakerDraft, SpeakerRow,
};

/// In-process `MeetingStore` fake. Used by `panops-core`'s own tests and
/// by engine integration tests where opening a real SQLite DB would be
/// unnecessary friction.
pub struct InMemoryMeetingStore {
    inner: Mutex<InMemoryMeetingInner>,
}

struct InMemoryMeetingInner {
    segments: std::collections::HashMap<i64, SegmentRow>,
    screenshots: std::collections::HashMap<i64, ScreenshotRow>,
    speakers: std::collections::HashMap<i64, SpeakerRow>,
    next_id: i64,
}

impl Default for InMemoryMeetingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryMeetingStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(InMemoryMeetingInner {
                segments: std::collections::HashMap::new(),
                screenshots: std::collections::HashMap::new(),
                speakers: std::collections::HashMap::new(),
                next_id: 1,
            }),
        }
    }

    fn next_id(inner: &mut InMemoryMeetingInner) -> i64 {
        let id = inner.next_id;
        inner.next_id += 1;
        id
    }
}

impl MeetingStore for InMemoryMeetingStore {
    fn create_segment(&self, draft: SegmentDraft) -> Result<SegmentRow, MeetingStoreError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| MeetingStoreError::sql("mutex poisoned"))?;
        let id = Self::next_id(&mut inner);
        let row = SegmentRow {
            id,
            meeting_id: draft.meeting_id,
            start_ms: draft.start_ms,
            end_ms: draft.end_ms,
            text: draft.text,
            language: draft.language,
            confidence: draft.confidence,
            speaker_id: draft.speaker_id,
            source: draft.source,
        };
        inner.segments.insert(id, row.clone());
        Ok(row)
    }

    fn list_segments(&self, meeting_id: &str) -> Result<Vec<SegmentRow>, MeetingStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| MeetingStoreError::sql("mutex poisoned"))?;
        let mut segments: Vec<SegmentRow> = inner
            .segments
            .values()
            .filter(|s| s.meeting_id == meeting_id)
            .cloned()
            .collect();
        segments.sort_by_key(|s| s.start_ms);
        Ok(segments)
    }

    fn create_screenshot(
        &self,
        draft: ScreenshotDraft,
    ) -> Result<ScreenshotRow, MeetingStoreError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| MeetingStoreError::sql("mutex poisoned"))?;
        let id = Self::next_id(&mut inner);
        let row = ScreenshotRow {
            id,
            meeting_id: draft.meeting_id,
            timestamp_ms: draft.timestamp_ms,
            path: draft.path,
            feature_print: draft.feature_print,
            caption: draft.caption,
        };
        inner.screenshots.insert(id, row.clone());
        Ok(row)
    }

    fn list_screenshots(&self, meeting_id: &str) -> Result<Vec<ScreenshotRow>, MeetingStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| MeetingStoreError::sql("mutex poisoned"))?;
        let mut screenshots: Vec<ScreenshotRow> = inner
            .screenshots
            .values()
            .filter(|s| s.meeting_id == meeting_id)
            .cloned()
            .collect();
        screenshots.sort_by_key(|s| s.timestamp_ms);
        Ok(screenshots)
    }

    fn create_speaker(&self, draft: SpeakerDraft) -> Result<SpeakerRow, MeetingStoreError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| MeetingStoreError::sql("mutex poisoned"))?;
        let id = Self::next_id(&mut inner);
        let row = SpeakerRow {
            id,
            meeting_id: draft.meeting_id,
            label: draft.label,
            embedding: draft.embedding,
        };
        inner.speakers.insert(id, row.clone());
        Ok(row)
    }

    fn list_speakers(&self, meeting_id: &str) -> Result<Vec<SpeakerRow>, MeetingStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| MeetingStoreError::sql("mutex poisoned"))?;
        Ok(inner
            .speakers
            .values()
            .filter(|s| s.meeting_id == meeting_id)
            .cloned()
            .collect())
    }

    fn get_speaker(&self, id: i64) -> Result<SpeakerRow, MeetingStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| MeetingStoreError::sql("mutex poisoned"))?;
        inner
            .speakers
            .get(&id)
            .cloned()
            .ok_or(MeetingStoreError::SpeakerNotFound { id })
    }

    fn update_speaker_label(&self, id: i64, label: &str) -> Result<SpeakerRow, MeetingStoreError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| MeetingStoreError::sql("mutex poisoned"))?;
        let speaker = inner
            .speakers
            .get_mut(&id)
            .ok_or(MeetingStoreError::SpeakerNotFound { id })?;
        speaker.label = label.into();
        Ok(speaker.clone())
    }
}

#[cfg(test)]
mod meeting_store_fake_tests {
    use super::InMemoryMeetingStore;
    use crate::conformance::meeting_store::run_suite;

    #[test]
    fn in_memory_meeting_store_passes_conformance() {
        run_suite(&InMemoryMeetingStore::new());
    }
}
