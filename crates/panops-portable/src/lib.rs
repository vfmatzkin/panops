//! Portable AsrProvider adapter wrapping `whisper-rs`. Used everywhere
//! the Mac sidecar isn't (Linux, Windows, fallback on Mac).

pub mod model;

mod whisper_adapter;
pub use whisper_adapter::WhisperRsAsr;
