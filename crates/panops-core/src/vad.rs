//! Voice Activity Detection port. Real impl: `panops_portable::WhisperVad`.
//! Fake: `panops_core::conformance::fakes::KnownRegionsFake`.
//!
//! The trait is sync; async wrapping happens at the handler layer via
//! `tokio::task::spawn_blocking`. Matches the shape of every other port
//! (`AsrProvider`, `LlmProvider`, `Diarizer`, `NotesExporter`, `Storage`).

use thiserror::Error;

pub trait Vad: Send + Sync {
    /// Detect speech regions in PCM samples. `samples` is mono f32
    /// in `[-1.0, 1.0]`; `sample_rate` is the source sample rate (Hz).
    /// Adapters typically require 16 kHz; verify and surface as
    /// `VadError::InvalidAudio` if mismatched.
    fn detect_speech(
        &self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<SpeechRegion>, VadError>;

    /// Marker for the conformance harness; production impls leave the default.
    fn is_fake(&self) -> bool {
        false
    }
}

/// A continuous interval of detected speech in the source audio,
/// expressed in milliseconds since the start. Adapters MUST return
/// regions sorted by `start_ms`, with `start_ms < end_ms`, and with
/// no overlapping ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechRegion {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Domain error. NEVER derive `Serialize` (per AGENTS.md: domain
/// errors stay platform-agnostic; transport conversion lives in
/// `panops-protocol` behind the `domain-conversions` feature).
#[derive(Debug, Error)]
pub enum VadError {
    #[error("vad model: {0}")]
    Model(String),
    #[error("invalid audio: {0}")]
    InvalidAudio(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_region_eq_and_dedup() {
        let a = SpeechRegion {
            start_ms: 0,
            end_ms: 1000,
        };
        let b = SpeechRegion {
            start_ms: 0,
            end_ms: 1000,
        };
        let c = SpeechRegion {
            start_ms: 500,
            end_ms: 2000,
        };

        // Equal fields → PartialEq holds.
        assert_eq!(a, b);
        assert_ne!(a, c);

        // Eq: dedup on a Vec collapses duplicates.
        let mut regions = vec![a, b, c];
        regions.dedup();
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0], a);
        assert_eq!(regions[1], c);
    }

    #[test]
    fn vad_error_display_includes_kind() {
        let e = VadError::Model("model load failed".into());
        assert!(format!("{e}").contains("vad model"));
        let e = VadError::InvalidAudio("not 16k".into());
        assert!(format!("{e}").contains("invalid audio"));
    }

    #[test]
    fn vad_error_io_via_from() {
        let io: std::io::Error = std::io::Error::other("disk full");
        let e: VadError = io.into();
        assert!(matches!(e, VadError::Io(..)));
    }
}
