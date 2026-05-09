//! Slice 06 — `ipc.meeting.set_language` updates the row and the
//! change survives a re-fetch.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::Meeting;
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_language_updates_row() {
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
    let id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![json!({"language":"en"})],
    )
    .await
    .expect("start");

    let updated: Meeting = ClientT::request(
        &client,
        "ipc.meeting.set_language",
        rpc_params![json!({"id":id.clone(),"language":"es"})],
    )
    .await
    .expect("set_language");

    assert_eq!(updated.language, "es");

    // Round-trip via storage too.
    let row = storage.get_meeting(&id).unwrap();
    assert_eq!(row.language, "es");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
