//! `ScreenCaptureKitCapture` conformance via the Rust fake-sidecar stub.
//!
//! Unlike the WhisperKit conformance test (which self-skips without a built
//! Swift sidecar), this runs in plain CI: the adapter drives the
//! `fake-capture-sidecar` `[[bin]]` test double, so the spawn / stdio /
//! respawn / `SessionNotFound` machinery is exercised without ScreenCaptureKit
//! or a TCC grant. The real Swift sidecar is validated only by the manual
//! Mac smoke.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};

use panops_core::capture::{Capture, CaptureConfig, CaptureError};
use panops_core::conformance::capture::run_suite_with;
use panops_mac::ScreenCaptureKitCapture;

fn fake_sidecar() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-capture-sidecar"))
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-mac")
        .join("tests/fixtures")
}

fn mode_env(mode: &str) -> Vec<(String, String)> {
    vec![("PANOPS_FAKE_SIDECAR_MODE".to_string(), mode.to_string())]
}

#[test]
fn screencapturekit_capture_passes_conformance() {
    let adapter = ScreenCaptureKitCapture::new(fake_sidecar());
    // The real adapter must report `is_fake() == false` even when its
    // sidecar is stubbed for CI.
    run_suite_with(&adapter, &fixtures_dir(), false);
}

#[test]
fn drop_pipe_surfaces_sidecar_error_and_respawns() {
    let adapter = ScreenCaptureKitCapture::with_env(fake_sidecar(), mode_env("drop_pipe"));
    let dir = tempfile::tempdir().expect("temp dir");
    let config = CaptureConfig::default();

    // The sidecar dies before acking; the adapter surfaces a Sidecar error.
    let err = adapter
        .start_capture("m1", dir.path(), &config)
        .expect_err("start should fail when the sidecar drops the pipe");
    assert!(matches!(err, CaptureError::Sidecar(_)), "got {err:?}");

    // The slot was cleared, so a second call respawns rather than wedging on
    // the dead pipe. The fresh child drops the pipe too, so we still get a
    // clean Sidecar error — never a hang or a stale-pipe panic.
    let err2 = adapter
        .start_capture("m2", dir.path(), &config)
        .expect_err("second start should also respawn-then-fail");
    assert!(matches!(err2, CaptureError::Sidecar(_)), "got {err2:?}");
}

#[test]
fn wire_unknown_session_maps_to_session_not_found() {
    let adapter = ScreenCaptureKitCapture::with_env(fake_sidecar(), mode_env("unknown_session"));
    let dir = tempfile::tempdir().expect("temp dir");
    let config = CaptureConfig::default();

    // Start acks normally, so the live-session map check passes and the
    // adapter reaches the sidecar — which reports the wire-level -32004.
    let session = adapter
        .start_capture("m1", dir.path(), &config)
        .expect("start should ack");
    let err = adapter
        .stop_capture(&session)
        .expect_err("stop should surface the sidecar's unknown-session error");
    assert!(
        matches!(err, CaptureError::SessionNotFound(_)),
        "got {err:?}"
    );
}
