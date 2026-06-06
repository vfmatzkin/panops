//! panops-core: domain types and ports. Zero platform code.

pub mod asr;
pub mod capture;
pub mod conformance;
pub mod diar;
pub mod exporter;
pub mod llm;
pub mod meeting_store;
pub mod merge;
pub mod notes;
pub mod segment;
pub mod storage;
pub mod vad;
pub mod wer;

pub use asr::{AsrError, AsrProvider};
pub use capture::{
    AudioSources, Capture, CaptureConfig, CaptureError, CaptureResult, CaptureSession,
};
pub use diar::{DiarError, Diarizer, SpeakerTurn};
pub use exporter::{ExportArtifact, ExportError, NotesExporter};
pub use llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};
pub use meeting_store::{
    MeetingStore, MeetingStoreError, ScreenshotDraft, ScreenshotRow, SegmentDraft, SegmentRow,
    SpeakerDraft, SpeakerRow,
};
pub use merge::merge_speaker_turns;
pub use notes::dialect::MarkdownDialect;
pub use notes::error::NotesError;
pub use notes::pipeline::NotesGenerator;
pub use segment::{Segment, Transcript};
pub use storage::{Meeting, MeetingDraft, MeetingSummary, Note, NoteDraft, Storage, StorageError};
pub use vad::{SpeechRegion, Vad, VadError};
