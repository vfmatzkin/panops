//! Slice 06 — `ipc.meeting.list` returns the rows in storage.
//! The slice-05 `ipc_meeting_list_returns_empty` still covers the
//! empty case (storage starts empty in that test); this one inserts
//! via the `Storage` port directly (since `meeting.start` is a later
//! task) and asserts the row surfaces over IPC.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownRegionsFake, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_core::storage::MeetingDraft;
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::MeetingSummary;
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_list_returns_rows_inserted_via_storage() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();

    // Insert two meetings BEFORE starting the server. Order matters:
    // started_at DESC, so "B" (later) should come first.
    storage
        .create_meeting(MeetingDraft {
            id: "a".into(),
            title: "A".into(),
            started_at: "2026-05-01T10:00:00+00:00".into(),
            language: "en".into(),
            dir_path: data_dir.join("meetings/a").to_string_lossy().into_owned(),
        })
        .unwrap();
    storage
        .create_meeting(MeetingDraft {
            id: "b".into(),
            title: "B".into(),
            started_at: "2026-05-03T10:00:00+00:00".into(),
            language: "en".into(),
            dir_path: data_dir.join("meetings/b").to_string_lossy().into_owned(),
        })
        .unwrap();

    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage.clone(),
        data_dir,
        Arc::new(TranscriptFileFake::default()),
        Arc::new(KnownTurnsFake),
        Arc::new(FakeNotesExporter),
        Arc::new(KnownRegionsFake::new()),
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
    let result: Vec<MeetingSummary> = ClientT::request(&client, "ipc.meeting.list", rpc_params![])
        .await
        .expect("call meeting.list");

    assert_eq!(result.len(), 2, "expected two rows, got {result:?}");
    // Order is started_at DESC: B (2026-05-03) before A (2026-05-01).
    assert_eq!(result[0].id, "b");
    assert_eq!(result[0].title, "B");
    assert_eq!(result[1].id, "a");
    assert_eq!(result[1].title, "A");
    // In-progress meetings render duration_ms as 0 (not Option<u64>).
    assert_eq!(result[0].duration_ms, 0);

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
