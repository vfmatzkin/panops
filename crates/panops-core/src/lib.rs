//! panops-core: domain types and ports. Zero platform code.

pub mod asr;
pub mod conformance;
pub mod diar;
pub mod exporter;
pub mod llm;
pub mod merge;
pub mod notes;
pub mod segment;
pub mod wer;

pub use asr::{AsrError, AsrProvider};
pub use diar::{DiarError, Diarizer, SpeakerTurn};
pub use exporter::{ExportArtifact, ExportError, NotesExporter};
pub use llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};
pub use merge::merge_speaker_turns;
pub use notes::dialect::MarkdownDialect;
pub use notes::error::NotesError;
pub use notes::pipeline::NotesGenerator;
pub use segment::{Segment, Transcript};
