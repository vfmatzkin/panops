use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::Transcript;

#[derive(Debug, Error)]
pub enum AsrError {
    #[error("audio file not found: {0}")]
    AudioNotFound(PathBuf),
    #[error("invalid audio: {0}")]
    InvalidAudio(String),
    #[error("model error: {0}")]
    Model(String),
    #[error("transcription failed: {0}")]
    Transcription(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub trait AsrProvider {
    /// Transcribe a complete audio file. Blocking, file-based.
    /// `language_hint` is `None` for auto-detect, `Some("en")` to force.
    fn transcribe_full(
        &self,
        audio_path: &Path,
        language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError>;

    /// Marker for the conformance harness; production impls leave the default.
    fn is_fake(&self) -> bool {
        false
    }
}
