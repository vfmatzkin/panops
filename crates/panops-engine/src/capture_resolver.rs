//! Resolve which `Capture` impl the engine uses at runtime.
//!
//! For slice 11, no macOS ScreenCaptureKit sidecar exists yet (that's the
//! live-capture path gated on manual Mac smoke). We return `FakeCapture`
//! when `PANOPS_TEST_CAPTURE=1` is set (for unit tests and integration tests).
//!
//! For non-test builds without the env var, we return a clear error since
//! the real adapter is pending (mirrors `asr_resolver`'s pattern).
//!
//! Once `apps/panops-capture-mac/` exists, this resolver will mirror
//! `asr_resolver`:
//!   - macOS: check `PANOPS_CAPTURE_SIDECAR_BIN` env var; if set and executable,
//!     use `ScreenCaptureKitCapture` sidecar adapter.
//!   - Otherwise: fall back to error in production.
//!
//! Design: `docs/superpowers/specs/2026-06-05-slice-11-live-capture-design.md`.

use std::sync::{Arc, OnceLock};

use panops_core::capture::Capture;

/// Cached capture adapter. `OnceLock` ensures the same instance is returned
/// on every call, so `start_capture` and `stop_capture` share the same
/// session map (critical for FakeCapture's internal HashMap).
static CAPTURE: OnceLock<Arc<dyn Capture + Send + Sync>> = OnceLock::new();

/// Resolve the capture adapter.
///
/// When `PANOPS_TEST_CAPTURE=1` is set, returns a cached `FakeCapture` for tests.
/// Otherwise, returns a capture that errors on `start_capture` since the
/// ScreenCaptureKit sidecar is not yet implemented.
pub fn pick_capture() -> Arc<dyn Capture + Send + Sync> {
    CAPTURE
        .get_or_init(|| {
            if std::env::var("PANOPS_TEST_CAPTURE").as_deref() == Ok("1") {
                Arc::new(panops_core::conformance::fakes::FakeCapture::new())
            } else {
                Arc::new(NotYetImplementedCapture)
            }
        })
        .clone()
}

/// Placeholder capture that returns a clear error for real/non-test use.
/// The macOS ScreenCaptureKit sidecar is not yet implemented.
struct NotYetImplementedCapture;

impl Capture for NotYetImplementedCapture {
    fn start_capture(
        &self,
        _meeting_id: &str,
        _meeting_dir: &std::path::Path,
        _config: &panops_core::capture::CaptureConfig,
    ) -> Result<panops_core::capture::CaptureSession, panops_core::capture::CaptureError> {
        Err(panops_core::capture::CaptureError::Capture(
            "live capture not yet implemented — ScreenCaptureKit sidecar pending".into(),
        ))
    }

    fn stop_capture(
        &self,
        _session: &panops_core::capture::CaptureSession,
    ) -> Result<panops_core::capture::CaptureResult, panops_core::capture::CaptureError> {
        Err(panops_core::capture::CaptureError::Capture(
            "live capture not yet implemented — ScreenCaptureKit sidecar pending".into(),
        ))
    }

    fn is_fake(&self) -> bool {
        false
    }
}
