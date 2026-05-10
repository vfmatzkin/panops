//! Slice 06 — `ipc.meeting.get` returns the full `Meeting` shape
//! (including `dir_path` and `language`). Unknown id surfaces as
//! `InputNotFound` over the wire.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::{ClientT, Error as ClientError};
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownRegionsFake, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::Meeting;
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_get_returns_all_fields() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage,
        data_dir,
        Arc::new(TranscriptFileFake::default()),
        Arc::new(KnownTurnsFake),
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
    let id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"X","language":"es"})],
    )
    .await
    .expect("start");

    let m: Meeting = ClientT::request(
        &client,
        "ipc.meeting.get",
        rpc_params![json!({"id":id.clone()})],
    )
    .await
    .expect("get");

    assert_eq!(m.id, id);
    assert_eq!(m.title, "X");
    assert_eq!(m.language, "es");
    assert!(m.ended_at.is_none());
    assert!(m.duration_ms.is_none());
    assert!(m.dir_path.contains("/meetings/"));

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_get_unknown_id_is_input_not_found() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage,
        data_dir,
        Arc::new(TranscriptFileFake::default()),
        Arc::new(KnownTurnsFake),
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
    let err = ClientT::request::<Meeting, _>(
        &client,
        "ipc.meeting.get",
        rpc_params![json!({"id":"nope"})],
    )
    .await
    .expect_err("unknown id must error");

    let ClientError::Call(call_err) = err else {
        panic!("expected Call error, got {err:?}");
    };
    let data: serde_json::Value =
        serde_json::from_str(call_err.data().unwrap().get()).expect("data is JSON");
    assert_eq!(data["kind"], "input_not_found");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
