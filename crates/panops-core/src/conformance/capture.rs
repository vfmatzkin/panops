//! Conformance harness for [`crate::capture::Capture`] adapters.
//!
//! Every Capture impl (real `ScreenCaptureKitCapture`, fake `FakeCapture`)
//! must pass this same suite. The harness asserts the contract documented
//! on the trait:
//!
//! - `start_capture` returns a session with the correct meeting_id.
//! - `stop_capture` returns paths to audio + screenshots.
//! - Audio path is a valid WAV file (parses with `hound`).
//! - Screenshots are valid JPEGs (paths exist; contents not verified).
//! - `is_fake` marker is set correctly.

use std::path::Path;

use crate::capture::{AudioSources, Capture, CaptureConfig, CaptureError, CaptureSession};

/// Run the full conformance suite against a `Capture` implementation.
///
/// Creates a fresh temp directory for each test to avoid mutating fixtures.
/// Screenshots are read from `fixtures_dir` (if needed by the adapter),
/// but all output goes to a temp location that is cleaned up after the test.
pub fn run_suite<C: Capture>(adapter: &C, fixtures_dir: &Path) {
    run_suite_with(adapter, fixtures_dir, true); // Fakes should return true
}

/// Like [`run_suite`] but asserts a specific `is_fake()` value.
///
/// The in-process `FakeCapture` passes `true`; a real adapter exercised
/// against a fake sidecar binary (e.g. `ScreenCaptureKitCapture`) passes
/// `false` — it must not opt out of the production marker just because its
/// sidecar is stubbed for CI.
pub fn run_suite_with<C: Capture>(adapter: &C, fixtures_dir: &Path, expected_is_fake: bool) {
    start_returns_session(adapter, fixtures_dir);
    stop_returns_valid_audio(adapter, fixtures_dir);
    stop_returns_screenshot_paths(adapter, fixtures_dir);
    stop_track_presence_matches_sources(adapter);
    record_video_writes_recording_mov(adapter);
    stop_session_not_found(adapter);
    is_fake_marker(adapter, expected_is_fake);
}

fn temp_meeting_dir() -> std::path::PathBuf {
    // Create temp dir under workspace target/ so FakeCapture can find
    // fixtures by walking ancestors (meeting_dir.ancestors() must include
    // workspace root with tests/fixtures/screenshots).
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2) // panops-core is at crates/panops-core, nth(2) is workspace root
        .expect("workspace root")
        .to_path_buf();

    let tmp_dir = workspace_root.join("target").join("tmp");
    std::fs::create_dir_all(&tmp_dir).expect("create target/tmp");

    tmp_dir.join(format!(
        "capture_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn start_returns_session<C: Capture>(adapter: &C, _fixtures_dir: &Path) {
    let meeting_id = "test_meeting_001";
    let meeting_dir = temp_meeting_dir();
    std::fs::create_dir_all(&meeting_dir).expect("create temp meeting dir");

    let config = CaptureConfig::default();
    let session = adapter
        .start_capture(meeting_id, &meeting_dir, &config)
        .expect("start_capture should succeed");
    assert_eq!(session.meeting_id, meeting_id);
    assert!(session.started_at_ms > 0, "started_at_ms should be set");

    // Clean up the session.
    let _ = adapter.stop_capture(&session);

    // Clean up temp dir.
    let _ = std::fs::remove_dir_all(&meeting_dir);
}

/// Assert that a path is a valid 16 kHz mono WAV and return its sample count.
fn assert_wav_16k_mono(path: &Path) -> u64 {
    assert!(path.exists(), "audio_path {} should exist", path.display());
    let reader = hound::WavReader::open(path).expect("audio should be valid WAV");
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, 16_000, "audio must be 16 kHz");
    assert_eq!(spec.channels, 1, "audio must be mono");
    reader.len() as u64
}

fn stop_returns_valid_audio<C: Capture>(adapter: &C, _fixtures_dir: &Path) {
    let meeting_id = "test_meeting_audio";
    let meeting_dir = temp_meeting_dir();
    std::fs::create_dir_all(&meeting_dir).expect("create temp meeting dir");

    // Default config is SystemAndMic, so both tracks must be present.
    let config = CaptureConfig::default();
    let session = adapter
        .start_capture(meeting_id, &meeting_dir, &config)
        .expect("start_capture should succeed");

    let result = adapter
        .stop_capture(&session)
        .expect("stop_capture should succeed");

    let system = result
        .system_audio_path
        .as_deref()
        .expect("SystemAndMic must yield a system track");
    let mic = result
        .mic_audio_path
        .as_deref()
        .expect("SystemAndMic must yield a mic track");
    let samples = assert_wav_16k_mono(system);
    assert_wav_16k_mono(mic);

    // Duration should match what the adapter reports.
    assert!(result.duration_ms > 0, "duration_ms should be positive");
    let computed_ms = (samples * 1000) / 16_000;
    // Allow small tolerance for rounding differences.
    let diff = (result.duration_ms as i64 - computed_ms as i64).abs();
    assert!(
        diff < 100,
        "duration mismatch: reported {} vs computed {}",
        result.duration_ms,
        computed_ms
    );

    // Clean up temp dir.
    let _ = std::fs::remove_dir_all(&meeting_dir);
}

fn stop_track_presence_matches_sources<C: Capture>(adapter: &C) {
    let cases = [
        (AudioSources::SystemOnly, true, false),
        (AudioSources::MicOnly, false, true),
        (AudioSources::SystemAndMic, true, true),
    ];
    for (sources, want_system, want_mic) in cases {
        let meeting_dir = temp_meeting_dir();
        std::fs::create_dir_all(&meeting_dir).expect("create temp meeting dir");
        let config = CaptureConfig {
            audio_sources: sources,
            ..CaptureConfig::default()
        };
        let session = adapter
            .start_capture("presence_test", &meeting_dir, &config)
            .expect("start_capture should succeed");
        let result = adapter.stop_capture(&session).expect("stop_capture");
        assert_eq!(
            result.system_audio_path.is_some(),
            want_system,
            "system track presence for {sources:?}"
        );
        assert_eq!(
            result.mic_audio_path.is_some(),
            want_mic,
            "mic track presence for {sources:?}"
        );
        let _ = std::fs::remove_dir_all(&meeting_dir);
    }
}

fn stop_returns_screenshot_paths<C: Capture>(adapter: &C, _fixtures_dir: &Path) {
    let meeting_id = "test_meeting_screenshots";
    let meeting_dir = temp_meeting_dir();
    std::fs::create_dir_all(&meeting_dir).expect("create temp meeting dir");

    let config = CaptureConfig::default();
    let session = adapter
        .start_capture(meeting_id, &meeting_dir, &config)
        .expect("start_capture should succeed");

    let result = adapter
        .stop_capture(&session)
        .expect("stop_capture should succeed");

    // Screenshots may be empty (fast stop) or have at least one.
    // Each path must exist on disk.
    for path in &result.screenshot_paths {
        assert!(path.exists(), "screenshot {} should exist", path.display());
    }

    // Clean up temp dir.
    let _ = std::fs::remove_dir_all(&meeting_dir);
}

fn record_video_writes_recording_mov<C: Capture>(adapter: &C) {
    let meeting_id = "test_meeting_video";
    let meeting_dir = temp_meeting_dir();
    std::fs::create_dir_all(&meeting_dir).expect("create temp meeting dir");

    let config = CaptureConfig {
        record_video: true,
        ..CaptureConfig::default()
    };
    let session = adapter
        .start_capture(meeting_id, &meeting_dir, &config)
        .expect("start_capture should succeed");

    adapter
        .stop_capture(&session)
        .expect("stop_capture should succeed");

    let video_path = meeting_dir.join("recording.mov");
    assert!(
        video_path.exists(),
        "record_video=true must write {}",
        video_path.display()
    );

    let _ = std::fs::remove_dir_all(&meeting_dir);
}

fn stop_session_not_found<C: Capture>(adapter: &C) {
    let fake_session = CaptureSession {
        meeting_id: "nonexistent_session".into(),
        started_at_ms: 0,
    };
    let err = adapter
        .stop_capture(&fake_session)
        .expect_err("stop_capture of unknown session should fail");
    match err {
        CaptureError::SessionNotFound(id) => {
            assert_eq!(id, "nonexistent_session");
        }
        other => panic!("expected SessionNotFound, got {other:?}"),
    }
}

fn is_fake_marker<C: Capture>(adapter: &C, expected_is_fake: bool) {
    // The trait default is `false`. Fakes should override to `true`.
    // This test asserts that the marker is set correctly for the adapter.
    assert_eq!(
        adapter.is_fake(),
        expected_is_fake,
        "is_fake() should return {} for this adapter",
        expected_is_fake
    );
}
