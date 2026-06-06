//! Resolve which `Capture` impl the engine uses at runtime.
//!
//! For slice 11, no macOS ScreenCaptureKit sidecar exists yet (that's the
//! live-capture path gated on manual Mac smoke). We return `FakeCapture`
//! unconditionally so the IPC handlers are unit-testable and CI-verifiable.
//!
//! Once `apps/panops-capture-mac/` exists, this resolver will mirror
//! `asr_resolver`:
//!   - macOS: check `PANOPS_CAPTURE_SIDECAR_BIN` env var; if set and executable,
//!     use `ScreenCaptureKitCapture` sidecar adapter.
//!   - Otherwise: fall back to `FakeCapture` (tests) or error in production.
//!
//! Design: `docs/superpowers/specs/2026-06-05-slice-11-live-capture-design.md`.

use std::sync::Arc;

use panops_core::capture::Capture;

/// Resolve the capture adapter. For slice 11 scaffolding, returns
/// `FakeCapture` unconditionally (no real ScreenCaptureKit adapter yet).
pub fn pick_capture() -> Arc<dyn Capture + Send + Sync> {
    // Slice 11 scaffolding: no macOS sidecar exists yet. Return FakeCapture
    // so the IPC handlers are unit-testable. The real ScreenCaptureKit adapter
    // will be gated on a manual Mac smoke test and the PANOPS_CAPTURE_SIDECAR_BIN
    // env var (mirroring asr_resolver).
    Arc::new(panops_core::conformance::fakes::FakeCapture::new())
}
