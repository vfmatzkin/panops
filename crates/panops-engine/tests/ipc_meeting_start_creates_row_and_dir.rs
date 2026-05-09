//! Slice 06 — `ipc.meeting.start` creates a meeting row, an on-disk
//! directory, and a `screenshots/` subdir. Returns the new id as a
//! plain string. Data-plane only (no capture coupling); Anchor B
//! layers ScreenCaptureKit on top.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_start_creates_row_directory_and_screenshots_subdir() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage.clone(),
        data_dir.clone(),
        Arc::new(TranscriptFileFake),
        Arc::new(KnownTurnsFake),
        Arc::new(FakeNotesExporter),
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
    let id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Daily","language":"en"})],
    )
    .await
    .expect("call meeting.start");

    assert!(!id.is_empty(), "id should be non-empty");

    // Row exists with the right title + language; started_at server-set.
    let m = storage.get_meeting(&id).expect("row should exist");
    assert_eq!(m.title, "Daily");
    assert_eq!(m.language, "en");
    assert!(m.ended_at.is_none());
    assert!(!m.started_at.is_empty(), "started_at server-set");

    // Filesystem layout.
    let meeting_dir = data_dir.join("meetings").join(&id);
    assert!(
        meeting_dir.exists(),
        "meeting dir should exist: {meeting_dir:?}"
    );
    assert!(
        meeting_dir.join("screenshots").exists(),
        "screenshots subdir should exist"
    );

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_start_with_empty_config_uses_defaults() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage.clone(),
        data_dir,
        Arc::new(TranscriptFileFake),
        Arc::new(KnownTurnsFake),
        Arc::new(FakeNotesExporter),
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
    let id: String = ClientT::request(&client, "ipc.meeting.start", rpc_params![json!({})])
        .await
        .expect("call meeting.start with empty config");
    let m = storage.get_meeting(&id).unwrap();
    assert_eq!(m.title, "", "default title is empty string");
    assert_eq!(m.language, "auto", "default language is 'auto'");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
