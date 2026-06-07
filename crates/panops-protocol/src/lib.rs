//! Wire types for panops IPC.
//!
//! `panops-protocol` is transport-only: serde-derived request/response/event
//! types and the `IpcError` taxonomy that flows over JSON-RPC and WebSocket
//! events. No engine logic, no I/O, no transport code.
//!
//! `panops-core` does NOT depend on this crate. The reverse direction is
//! gated behind the `domain-conversions` feature so non-Rust consumers
//! (e.g., a future Swift client codegen target) can build the wire types
//! without pulling the domain crate.

pub mod error;
pub mod methods;

pub use error::IpcError;
pub use methods::{
    AudioSourcesWire, Event, JobAccepted, JobDoneEvent, JobErrorEvent, JobProgressEvent, Meeting,
    MeetingConfig, MeetingSummary, NotesDialect, NotesGenerateParams, NotesGenerateResult,
    RecordingAccepted, RecordingProgressEvent, RecordingStartParams, RecordingStopParams,
    RecordingStopped, ScreenshotEvent,
};
