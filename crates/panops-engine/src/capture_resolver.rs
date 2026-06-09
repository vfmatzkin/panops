//! Resolve which `Capture` impl the engine uses at runtime.
//!
//! On macOS, resolve the ScreenCaptureKit sidecar
//! (`panops_mac::ScreenCaptureKitCapture`) for live capture in this order:
//! (1) `PANOPS_CAPTURE_SIDECAR_BIN` if set to an executable file (dev/CI gate);
//! else (2) a `panops-capture-mac` binary sitting next to the engine in a
//! packaged `.app` bundle (production, slice 11). If neither resolves, test
//! builds can opt into `FakeCapture` via `PANOPS_TEST_CAPTURE=1`; otherwise the
//! resolver falls back to `NotYetImplementedCapture`.
//!
//! Design: `docs/superpowers/specs/2026-06-07-slice-11-live-capture-design.md`.

use std::sync::{Arc, OnceLock};

use panops_core::capture::Capture;

/// Cached capture adapter. `OnceLock` ensures the same instance is returned
/// on every call, so `start_capture` and `stop_capture` share the same
/// session map (critical for FakeCapture's internal HashMap).
static CAPTURE: OnceLock<Arc<dyn Capture + Send + Sync>> = OnceLock::new();

/// Cached sidecar binary path, resolved by the same rules as the adapter.
/// `None` when no sidecar is available (non-macOS, or env + sibling both
/// miss). Separated from [`pick_capture`] so the engine's post-recording
/// extract-screenshots path can invoke the sidecar directly without
/// holding a `Capture` adapter.
static SIDECAR_BINARY: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();

/// Resolve the capture adapter.
///
/// When `PANOPS_TEST_CAPTURE=1` is set, returns a cached `FakeCapture` for tests.
/// On macOS with `PANOPS_CAPTURE_SIDECAR_BIN` set to an executable sidecar,
/// returns the real `ScreenCaptureKitCapture` adapter.
pub fn pick_capture() -> Arc<dyn Capture + Send + Sync> {
    CAPTURE
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            {
                let sidecar = resolve_sidecar_binary();
                if let Some(sidecar) = sidecar {
                    tracing::info!(
                        sidecar = %sidecar.display(),
                        "selecting ScreenCaptureKit capture sidecar"
                    );
                    return Arc::new(panops_mac::ScreenCaptureKitCapture::new(sidecar));
                }
            }
            if std::env::var("PANOPS_TEST_CAPTURE").as_deref() == Ok("1") {
                Arc::new(panops_core::conformance::fakes::FakeCapture::new())
            } else {
                Arc::new(NotYetImplementedCapture)
            }
        })
        .clone()
}

/// Resolve the `panops-capture-mac` sidecar binary path. Cached in a
/// `OnceLock` so repeated calls (e.g. per-stop extraction) don't re-walk
/// the env + sibling lookup. `None` on non-macOS or when neither the
/// env override nor the packaged sibling resolves.
pub fn sidecar_binary() -> Option<std::path::PathBuf> {
    SIDECAR_BINARY
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            {
                resolve_sidecar_binary()
            }
            #[cfg(not(target_os = "macos"))]
            {
                None
            }
        })
        .clone()
}

/// Pure resolution: env override first, packaged sibling second. Split
/// out so both [`pick_capture`] and [`sidecar_binary`] share it without
/// duplicating the `#[cfg]` branches.
#[cfg(target_os = "macos")]
fn resolve_sidecar_binary() -> Option<std::path::PathBuf> {
    std::env::var_os("PANOPS_CAPTURE_SIDECAR_BIN")
        .and_then(|v| crate::sidecar_binary::executable_file(std::path::PathBuf::from(v)))
        .or_else(|| crate::sidecar_binary::sibling_of_engine("panops-capture-mac"))
}

/// Placeholder capture that returns a clear error for real/non-test use.
/// The macOS ScreenCaptureKit sidecar is now implemented; this fallback
/// applies only when the sidecar env var is not set or on non-macOS platforms.
struct NotYetImplementedCapture;

impl Capture for NotYetImplementedCapture {
    fn start_capture(
        &self,
        _meeting_id: &str,
        _meeting_dir: &std::path::Path,
        _config: &panops_core::capture::CaptureConfig,
    ) -> Result<panops_core::capture::CaptureSession, panops_core::capture::CaptureError> {
        Err(panops_core::capture::CaptureError::Capture(
            "live capture not available — PANOPS_CAPTURE_SIDECAR_BIN not set".into(),
        ))
    }

    fn stop_capture(
        &self,
        _session: &panops_core::capture::CaptureSession,
    ) -> Result<panops_core::capture::CaptureResult, panops_core::capture::CaptureError> {
        Err(panops_core::capture::CaptureError::Capture(
            "live capture not available — PANOPS_CAPTURE_SIDECAR_BIN not set".into(),
        ))
    }
}
