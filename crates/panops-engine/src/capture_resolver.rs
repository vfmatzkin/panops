//! Resolve which `Capture` impl the engine uses at runtime.
//!
//! On macOS, the ScreenCaptureKit sidecar adapter (`panops_mac::ScreenCaptureKitCapture`)
//! is wired via `PANOPS_CAPTURE_SIDECAR_BIN` env var for dev/CI, or via a
//! `panops-capture-mac` binary sitting next to the engine in a packaged `.app`
//! bundle (production, slice 11).
//!
//! Tiered resolution:
//!   1. macOS only: check `PANOPS_CAPTURE_SIDECAR_BIN` env var; if set to an
//!      executable file (dev/CI), use `ScreenCaptureKitCapture` sidecar adapter.
//!   2. If `PANOPS_TEST_CAPTURE=1` is set, return `FakeCapture` for tests.
//!   3. Otherwise, fall back to `NotYetImplementedCapture` ( Unix-only or
//!      macOS without sidecar binary).
//!
//! Design: `docs/superpowers/specs/2026-06-07-slice-11-live-capture-design.md`.

use std::sync::{Arc, OnceLock};

use panops_core::capture::Capture;

/// Cached capture adapter. `OnceLock` ensures the same instance is returned
/// on every call, so `start_capture` and `stop_capture` share the same
/// session map (critical for FakeCapture's internal HashMap).
static CAPTURE: OnceLock<Arc<dyn Capture + Send + Sync>> = OnceLock::new();

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
                if let Some(sidecar) = sidecar_binary() {
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

/// Validate the dev/CI-only `PANOPS_CAPTURE_SIDECAR_BIN` gate: the env var
/// must point to an executable file, else we fall back cleanly. Mirrors
/// `asr_resolver::sidecar_binary`.
#[cfg(target_os = "macos")]
fn sidecar_binary() -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let bin = std::env::var("PANOPS_CAPTURE_SIDECAR_BIN").ok()?;
    // Canonicalize before the metadata check to narrow the symlink-swap window
    // between validation and `Command::spawn`. Best-effort UX (clean fallback
    // on unset/bad input), not a security boundary — the path is local-only and
    // operator-set, so a sufficiently fast swap after canonicalize can still
    // race the spawn; the threat model accepts that.
    let path = std::fs::canonicalize(bin).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    // A regular file without exec bits would `spawn`-fail at runtime; reject up
    // front and fall back.
    if meta.permissions().mode() & 0o111 == 0 {
        return None;
    }
    Some(path)
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn sidecar_binary_rejects_invalid_paths() {
        // PANOPS_CAPTURE_SIDECAR_BIN is process-global; one test owns it and
        // checks both reject cases sequentially to avoid an intra-binary race.
        let original = std::env::var_os("PANOPS_CAPTURE_SIDECAR_BIN");

        // Non-existent path → None (canonicalize fails).
        unsafe { std::env::set_var("PANOPS_CAPTURE_SIDECAR_BIN", "/no/such/panops/sidecar") };
        assert_eq!(sidecar_binary(), None, "non-existent path must not resolve");

        // Existing but non-executable file → None (exec-bit check).
        let mut tmp = std::env::temp_dir();
        tmp.push("panops-capture-noexec-test");
        std::fs::write(&tmp, b"not executable").unwrap();
        unsafe { std::env::set_var("PANOPS_CAPTURE_SIDECAR_BIN", &tmp) };
        let got_noexec = sidecar_binary();
        let _ = std::fs::remove_file(&tmp);

        match original {
            Some(v) => unsafe { std::env::set_var("PANOPS_CAPTURE_SIDECAR_BIN", v) },
            None => unsafe { std::env::remove_var("PANOPS_CAPTURE_SIDECAR_BIN") },
        }
        assert_eq!(got_noexec, None, "non-executable file must not resolve");
    }
}
