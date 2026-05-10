//! Portable AsrProvider + Diarizer adapters. Used everywhere the
//! Mac sidecars aren't (Linux, Windows, fallback on Mac).

pub mod audio;
pub mod genai_llm;
pub mod markdown_exporter;
pub mod model;
pub mod rusqlite_storage;

mod sherpa_diarizer;
mod whisper_adapter;
pub mod whisper_vad;

pub use genai_llm::GenaiLlm;
pub use markdown_exporter::MarkdownExporter;
pub use rusqlite_storage::RusqliteStorage;
pub use sherpa_diarizer::SherpaDiarizer;
pub use whisper_adapter::WhisperRsAsr;
pub use whisper_vad::WhisperVad;
