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
