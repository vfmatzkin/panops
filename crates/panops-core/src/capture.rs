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

/// Screen target to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    /// A whole display. `display_id` 0 = primary.
    Display { display_id: u32 },
    /// A specific window by ScreenCaptureKit window id.
    Window { window_id: u32 },
    /// All windows of an app, by bundle id.
    App { bundle_id: String },
    /// A sub-rectangle of a display (origin + size, in display points).
    Region {
        display_id: u32,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    },
}

impl Default for CaptureTarget {
    fn default() -> Self {
        CaptureTarget::Display { display_id: 0 }
    }
}

/// Configuration for a capture session.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureConfig {
    /// Audio sources to capture.
    pub audio_sources: AudioSources,
    /// Whether to write a screen-video file at `<meeting_dir>/recording.mov`.
    pub record_video: bool,
    /// Screenshot sampling interval in milliseconds.
    pub screenshot_interval_ms: u64,
    /// Vision FeaturePrint cosine distance threshold for change detection.
    pub screenshot_threshold: f32,
    /// Screen target to capture. Defaults to full-display capture.
    pub capture_target: CaptureTarget,
    /// Output width in pixels. `None` = native (no downscale).
    pub width: Option<u32>,
    /// Output height in pixels. `None` = native. Set both or neither.
    pub height: Option<u32>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            audio_sources: AudioSources::SystemAndMic,
            record_video: false,
            screenshot_interval_ms: 500,
            screenshot_threshold: 0.15,
            capture_target: CaptureTarget::Display { display_id: 0 },
            width: None,
            height: None,
        }
    }
}

/// Handle returned by `start_capture`. Used to identify the session
/// for `stop_capture`. Carries the subset of the originating
/// [`CaptureConfig`] the stop path (and the engine above it) needs to
/// decide what to do with the captured artifacts — e.g. whether a
/// `recording.mov` was written that needs post-processing into
/// screenshots.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureSession {
    pub meeting_id: String,
    pub started_at_ms: u64,
    /// Screen target selected when the session was started.
    pub capture_target: CaptureTarget,
    /// Whether the session is muxing a screen-video file. The engine's
    /// stop path uses this to decide between live-screenshot delivery
    /// (no-video) and post-recording frame extraction (video).
    pub record_video: bool,
    /// Vision FeaturePrint cadence + threshold used by the live
    /// Screenshotter / the extractor. Persisted on the session so
    /// `stop_capture` (and the engine above it) can re-derive the
    /// extraction parameters without re-reading the originating config.
    pub screenshot_interval_ms: u64,
    pub screenshot_threshold: f32,
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
        assert!(!cfg.record_video);
        assert_eq!(cfg.screenshot_interval_ms, 500);
        assert!(cfg.screenshot_threshold > 0.0 && cfg.screenshot_threshold < 1.0);
        assert_eq!(cfg.capture_target, CaptureTarget::Display { display_id: 0 });
    }

    #[test]
    fn capture_target_default_is_display() {
        assert_eq!(
            CaptureTarget::default(),
            CaptureTarget::Display { display_id: 0 }
        );
    }

    #[test]
    fn capture_target_variants_construct() {
        let _ = CaptureTarget::Display { display_id: 0 };
        let _ = CaptureTarget::Window { window_id: 9 };
        let _ = CaptureTarget::App {
            bundle_id: "com.apple.Safari".into(),
        };
        let _ = CaptureTarget::Region {
            display_id: 0,
            x: 10,
            y: 20,
            w: 640,
            h: 480,
        };
    }

    #[test]
    fn capture_target_default_is_primary_display() {
        assert_eq!(
            CaptureTarget::default(),
            CaptureTarget::Display { display_id: 0 }
        );
    }

    #[test]
    fn capture_config_default_has_no_explicit_resolution() {
        let cfg = CaptureConfig::default();
        assert_eq!(cfg.width, None);
        assert_eq!(cfg.height, None);
        assert_eq!(cfg.capture_target, CaptureTarget::Display { display_id: 0 });
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
            capture_target: CaptureTarget::Display { display_id: 0 },
            record_video: true,
            screenshot_interval_ms: 500,
            screenshot_threshold: 0.15,
        };
        assert_eq!(s.meeting_id, "abc123");
        assert_eq!(s.started_at_ms, 1_700_000_000_000);
        assert_eq!(s.capture_target, CaptureTarget::Display { display_id: 0 });
        assert!(s.record_video);
        assert_eq!(s.screenshot_interval_ms, 500);
        assert!((s.screenshot_threshold - 0.15).abs() < f32::EPSILON);
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
