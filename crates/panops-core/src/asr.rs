use std::path::PathBuf;

use thiserror::Error;

use crate::Transcript;

#[derive(Debug, Error)]
pub enum AsrError {
    #[error("invalid audio: {0}")]
    InvalidAudio(String),
    #[error("model error: {0}")]
    Model(String),
    #[error("transcription failed: {0}")]
    Transcription(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Retained for backward-compat with callers that pass a path
    /// somewhere up the stack and want to surface a missing file.
    /// Internal-pipeline callers map to `InvalidAudio` instead.
    #[error("audio file not found: {0}")]
    AudioNotFound(PathBuf),
}

pub trait AsrProvider: Send + Sync {
    /// Transcribe a chunk of mono PCM samples. `sample_rate` is the
    /// source sample rate (Hz); adapters typically require 16 kHz
    /// and surface mismatches as `AsrError::InvalidAudio`.
    /// `language_hint` is `None` for per-call auto-detect (Whisper
    /// detects from the first ~30s of the chunk), `Some("en")` to
    /// force a specific BCP-47 language.
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError>;

    /// Marker for the conformance harness; production impls leave the default.
    fn is_fake(&self) -> bool {
        false
    }
}
