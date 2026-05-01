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
