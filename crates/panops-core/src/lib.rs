//! panops-core: domain types and ports. Zero platform code.

pub mod asr;
pub mod conformance;
pub mod diar;
pub mod merge;
pub mod segment;
pub mod wer;

pub use asr::{AsrError, AsrProvider};
pub use diar::{DiarError, Diarizer, SpeakerTurn};
pub use merge::merge_speaker_turns;
pub use segment::{Segment, Transcript};
