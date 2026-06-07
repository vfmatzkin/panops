//! Capture port for audio + screenshot capture.
//!
//! The trait is sync; async wrapping happens at the handler layer via
//! `tokio::task::spawn_blocking`. Matches the shape of the other ports
//! (`AsrProvider`, `Vad`, `Storage`).
//!
//! Real adapter: the forthcoming macOS ScreenCaptureKit sidecar
//! (slice 11 scaffolding; trait + fake + conformance land now,
//! real capture gated on manual Mac smoke).
//! Fake: `panops_core::conformance::fakes::FakeCapture`.

use std::path::PathBuf;

use thiserror::Error;

/// Audio sources to capture. Passed to `start_capture`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSources {
    /// System audio output only.
    SystemOnly,
    /// Microphone only.
    MicOnly,
    /// Both system audio and microphone, mixed.
    SystemAndMic,
}

/// Configuration for a capture session.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureConfig {
    /// Audio sources to capture.
    pub audio_sources: AudioSources,
    /// Screenshot sampling interval in milliseconds.
    pub screenshot_interval_ms: u64,
    /// Vision FeaturePrint cosine distance threshold for change detection.
    pub screenshot_threshold: f32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            audio_sources: AudioSources::SystemAndMic,
            screenshot_interval_ms: 500,
            screenshot_threshold: 0.15,
        }
    }
}

/// Handle returned by `start_capture`. Used to identify the session
/// for `stop_capture`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSession {
    pub meeting_id: String,
    pub started_at_ms: u64,
}

/// Result returned by `stop_capture`. Contains paths to captured artifacts.
///
/// Audio is delivered as two separate tracks (slice 11, Decision §2):
/// `system_audio_path` holds remote participants (SCStream `.audio`) and
/// `mic_audio_path` holds the local user (SCStream `.microphone`). Each is
/// `Some` exactly when its source was requested via `AudioSources`; at least
/// one is always `Some` for a successful capture. Both are 16 kHz mono WAVs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureResult {
    /// System-audio (remote) track; `None` when not requested (`MicOnly`).
    pub system_audio_path: Option<PathBuf>,
    /// Microphone (local) track; `None` when not requested (`SystemOnly`).
    pub mic_audio_path: Option<PathBuf>,
    /// Paths to captured screenshot JPEG files.
    pub screenshot_paths: Vec<PathBuf>,
    /// Duration of the recording in milliseconds.
    pub duration_ms: u64,
}

/// Domain error for capture operations. NEVER derive `Serialize` (per
/// AGENTS.md: domain errors stay platform-agnostic; transport conversion
/// lives in `panops-protocol` behind the `domain-conversions` feature).
#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("capture failed: {0}")]
    Capture(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sidecar error: {0}")]
    Sidecar(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Capture trait for audio + screenshot capture. One combined trait per
/// D2 (YAGNI: no pre-split for AudioCapture/VideoCapture).
///
/// Real adapters (ScreenCaptureKitCapture) spawn a Swift sidecar that
/// runs ScreenCaptureKit + AVFoundation. Fake adapters (FakeCapture)
/// yield synthetic PCM frames and pre-generated screenshot fixtures.
pub trait Capture: Send + Sync {
    /// Start capturing audio + screenshots for a meeting.
    /// Returns a session handle. Audio + screenshots stream as IPC events
    /// (implemented at the engine layer, not here).
    fn start_capture(
        &self,
        meeting_id: &str,
        meeting_dir: &std::path::Path,
        config: &CaptureConfig,
    ) -> Result<CaptureSession, CaptureError>;

    /// Stop capturing, finalize audio file, return paths.
    fn stop_capture(&self, session: &CaptureSession) -> Result<CaptureResult, CaptureError>;

    /// Marker for conformance harness; production impls leave the default.
    fn is_fake(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_config_default_values() {
        let cfg = CaptureConfig::default();
        assert_eq!(cfg.audio_sources, AudioSources::SystemAndMic);
        assert_eq!(cfg.screenshot_interval_ms, 500);
        assert!(cfg.screenshot_threshold > 0.0 && cfg.screenshot_threshold < 1.0);
    }

    #[test]
    fn audio_sources_equality() {
        assert_eq!(AudioSources::SystemOnly, AudioSources::SystemOnly);
        assert_ne!(AudioSources::SystemOnly, AudioSources::MicOnly);
        assert_ne!(AudioSources::MicOnly, AudioSources::SystemAndMic);
    }

    #[test]
    fn capture_session_fields() {
        let s = CaptureSession {
            meeting_id: "abc123".into(),
            started_at_ms: 1_700_000_000_000,
        };
        assert_eq!(s.meeting_id, "abc123");
        assert_eq!(s.started_at_ms, 1_700_000_000_000);
    }

    #[test]
    fn capture_result_paths() {
        let r = CaptureResult {
            system_audio_path: Some(PathBuf::from("/tmp/system.wav")),
            mic_audio_path: Some(PathBuf::from("/tmp/mic.wav")),
            screenshot_paths: vec![
                PathBuf::from("/tmp/screenshots/001.jpg"),
                PathBuf::from("/tmp/screenshots/002.jpg"),
            ],
            duration_ms: 60_000,
        };
        assert_eq!(
            r.system_audio_path
                .as_deref()
                .unwrap()
                .display()
                .to_string(),
            "/tmp/system.wav"
        );
        assert_eq!(
            r.mic_audio_path.as_deref().unwrap().display().to_string(),
            "/tmp/mic.wav"
        );
        assert_eq!(r.screenshot_paths.len(), 2);
    }

    #[test]
    fn capture_result_allows_single_track() {
        let r = CaptureResult {
            system_audio_path: Some(PathBuf::from("/tmp/system.wav")),
            mic_audio_path: None,
            screenshot_paths: vec![],
            duration_ms: 1_000,
        };
        assert!(r.system_audio_path.is_some());
        assert!(r.mic_audio_path.is_none());
    }

    #[test]
    fn capture_error_display() {
        let e = CaptureError::PermissionDenied("microphone".into());
        assert!(format!("{e}").contains("permission denied"));
        let e = CaptureError::Capture("device busy".into());
        assert!(format!("{e}").contains("capture failed"));
        let e = CaptureError::Sidecar("process exited".into());
        assert!(format!("{e}").contains("sidecar error"));
    }

    #[test]
    fn capture_error_io_from() {
        let io = std::io::Error::other("disk full");
        let e: CaptureError = io.into();
        assert!(matches!(e, CaptureError::Io(..)));
    }
}
