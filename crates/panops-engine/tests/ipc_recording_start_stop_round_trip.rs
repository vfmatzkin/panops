//! Slice 11 — `recording.start` → `recording.stop` IPC round-trip.
//!
//! Uses `FakeCapture` via PANOPS_TEST_CAPTURE=1 env var resolver. Verifies that:
//! - recording.start creates a session and persists it
//! - recording.stop looks up the real session (not a fabricated one)
//! - The shared capture instance is used (same FakeCapture via OnceLock)

mod common;

use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownRegionsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::{RecordingAccepted, RecordingStopped};
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

/// Set PANOPS_TEST_CAPTURE=1 before any tests run so the resolver
/// picks FakeCapture. This must happen before the OnceLock is initialized.
static TEST_CAPTURE_INIT: std::sync::Once = std::sync::Once::new();

fn ensure_test_capture() {
    TEST_CAPTURE_INIT.call_once(|| {
        // SAFETY: setting an env var before any tests run is safe; no other
        // thread can read it before initialization completes.
        unsafe {
            std::env::set_var("PANOPS_TEST_CAPTURE", "1");
        }
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recording_start_stop_round_trip() {
    ensure_test_capture();
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage.clone(),
        data_dir,
        Arc::new(TranscriptFileFake::default()),
        Arc::new(panops_core::conformance::fakes::KnownTurnsFake),
        Arc::new(FakeNotesExporter),
        Arc::new(KnownRegionsFake::default()),
    );

    let server_socket = socket.clone();
    let server_shutdown = shutdown_rx.clone();
    let server = tokio::spawn(async move {
        run_serve_in_process(&server_socket, services, Some(server_shutdown))
            .await
            .unwrap();
    });

    wait_for_socket(&socket).await;

    let client = uds_ws_client(&socket).await;

    // Create a meeting first (recording.start requires an existing meeting).
    let meeting_id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![serde_json::json!({
            "title": "Test Recording",
            "language": "en"
        })],
    )
    .await
    .expect("meeting.start");

    // Start recording.
    let accepted: RecordingAccepted = ClientT::request(
        &client,
        "ipc.recording.start",
        rpc_params![serde_json::json!({
            "meeting_id": meeting_id,
            "audio_sources": "system_and_mic",
            "screenshot_interval_ms": 500,
            "screenshot_threshold": 0.15
        })],
    )
    .await
    .expect("recording.start");

    assert_eq!(accepted.recording_id, meeting_id);

    // Stop recording after a brief pause.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let stopped: RecordingStopped = ClientT::request(
        &client,
        "ipc.recording.stop",
        rpc_params![serde_json::json!({
            "recording_id": meeting_id
        })],
    )
    .await
    .expect("recording.stop");

    // FakeCapture default config is SystemAndMic → both tracks present.
    assert!(stopped.system_audio_path.is_some(), "system track present");
    assert!(stopped.mic_audio_path.is_some(), "mic track present");
    assert!(stopped.duration_ms > 0, "duration should be positive");

    // Verify screenshot paths are present (FakeCapture copies fixtures).
    assert!(
        !stopped.screenshot_paths.is_empty(),
        "should have screenshots"
    );

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recording_start_with_audio_sources_wire() {
    ensure_test_capture();
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage.clone(),
        data_dir,
        Arc::new(TranscriptFileFake::default()),
        Arc::new(panops_core::conformance::fakes::KnownTurnsFake),
        Arc::new(FakeNotesExporter),
        Arc::new(KnownRegionsFake::default()),
    );

    let server_socket = socket.clone();
    let server_shutdown = shutdown_rx.clone();
    let server = tokio::spawn(async move {
        run_serve_in_process(&server_socket, services, Some(server_shutdown))
            .await
            .unwrap();
    });

    wait_for_socket(&socket).await;

    let client = uds_ws_client(&socket).await;

    // Create a meeting.
    let meeting_id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![serde_json::json!({
            "title": "Test Audio Sources"
        })],
    )
    .await
    .expect("meeting.start");

    // Start recording with explicit audio_sources = mic_only.
    let accepted: RecordingAccepted = ClientT::request(
        &client,
        "ipc.recording.start",
        rpc_params![serde_json::json!({
            "meeting_id": meeting_id,
            "audio_sources": "mic_only"
        })],
    )
    .await
    .expect("recording.start with audio_sources");

    assert_eq!(accepted.recording_id, meeting_id);

    // Stop recording.
    let stopped: RecordingStopped = ClientT::request(
        &client,
        "ipc.recording.stop",
        rpc_params![serde_json::json!({
            "recording_id": meeting_id
        })],
    )
    .await
    .expect("recording.stop");

    // audio_sources = mic_only → only the mic track is present.
    assert!(stopped.system_audio_path.is_none(), "no system track");
    assert!(stopped.mic_audio_path.is_some(), "mic track present");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recording_stop_unknown_session_returns_input_not_found() {
    ensure_test_capture();
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage.clone(),
        data_dir,
        Arc::new(TranscriptFileFake::default()),
        Arc::new(panops_core::conformance::fakes::KnownTurnsFake),
        Arc::new(FakeNotesExporter),
        Arc::new(KnownRegionsFake::default()),
    );

    let server_socket = socket.clone();
    let server_shutdown = shutdown_rx.clone();
    let server = tokio::spawn(async move {
        run_serve_in_process(&server_socket, services, Some(server_shutdown))
            .await
            .unwrap();
    });

    wait_for_socket(&socket).await;

    let client = uds_ws_client(&socket).await;

    // Try to stop a recording that was never started.
    let result: Result<RecordingStopped, _> = ClientT::request(
        &client,
        "ipc.recording.stop",
        rpc_params![serde_json::json!({
            "recording_id": "nonexistent_session"
        })],
    )
    .await;

    // Expect error with kind = input_not_found.
    let err = result.expect_err("recording.stop should fail for unknown session");
    // JSON-RPC error code -32000 with IpcError in data.
    assert!(err.to_string().contains("input_not_found") || err.to_string().contains("not found"));

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
