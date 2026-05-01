//! panops-core: domain types and ports. Zero platform code.

pub mod asr;
pub mod conformance;
pub mod segment;
pub mod wer;

pub use asr::{AsrError, AsrProvider};
pub use segment::{Segment, Transcript};
