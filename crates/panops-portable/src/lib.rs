//! Portable AsrProvider + Diarizer adapters. Used everywhere the
//! Mac sidecars aren't (Linux, Windows, fallback on Mac).

pub mod genai_llm;
pub mod markdown_exporter;
pub mod model;

mod sherpa_diarizer;
mod whisper_adapter;

pub use genai_llm::GenaiLlm;
pub use markdown_exporter::MarkdownExporter;
pub use sherpa_diarizer::SherpaDiarizer;
pub use whisper_adapter::WhisperRsAsr;
