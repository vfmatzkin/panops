//! WhisperKit ASR sidecar adapter. Slice 10 Task 6 fills this in.
//!
//! Placeholder type so the scaffold compiles; Task 6 replaces with
//! the real impl + `AsrProvider` trait implementation.

use std::path::PathBuf;

pub struct WhisperKitAsr {
    #[allow(dead_code)]
    binary: PathBuf,
}

impl WhisperKitAsr {
    pub fn new(binary: PathBuf) -> Self {
        Self { binary }
    }
}
