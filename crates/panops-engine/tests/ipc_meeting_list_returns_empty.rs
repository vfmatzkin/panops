//! Slice 05 — `meeting.list` returns `[]` (placeholder until SQLite #17 lands).

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::MeetingSummary;
use tempfile::tempdir;
use tokio::sync::watch;

use common::{uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_list_returns_empty_array() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let services = EngineServices {
        llm: Arc::new(MockLlm::default()),
        asr: Arc::new(TranscriptFileFake),
        diar: Arc::new(KnownTurnsFake),
        exporter: Arc::new(FakeNotesExporter),
    };

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
    assert!(result.is_empty(), "expected empty list, got {result:?}");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
