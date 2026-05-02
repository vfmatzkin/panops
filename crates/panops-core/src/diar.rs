use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerTurn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker_id: u32,
}

#[derive(Debug, Error)]
pub enum DiarError {
    #[error("audio file not found: {0}")]
    AudioNotFound(PathBuf),
    #[error("invalid audio: {0}")]
    InvalidAudio(String),
    #[error("model error: {0}")]
    Model(String),
    #[error("diarization failed: {0}")]
    Diarization(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub trait Diarizer: Send + Sync {
    /// Run speaker diarization on a complete audio file. Stateless.
    /// Returns turns ordered by `start_ms`, non-overlapping. Speaker IDs
    /// are stable within one call only; meaningless across calls.
    fn diarize(&self, audio_path: &Path) -> Result<Vec<SpeakerTurn>, DiarError>;

    /// Marker for the conformance harness; production impls leave the default.
    fn is_fake(&self) -> bool {
        false
    }
}
