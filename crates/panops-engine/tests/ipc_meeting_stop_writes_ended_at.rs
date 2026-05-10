//! Slice 06 — `ipc.meeting.stop` sets `ended_at` and computes
//! `duration_ms = ended_at - started_at`. Returns the updated row.
//! Unknown id surfaces as `InputNotFound` over the wire.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::client::Error as ClientError;
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
async fn meeting_stop_sets_ended_at_and_duration() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
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
    let id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"X"})],
    )
    .await
    .expect("start");

    // Brief sleep so duration_ms is observably > 0.
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;

    let stopped: Meeting =
        ClientT::request(&client, "ipc.meeting.stop", rpc_params![json!({"id":id})])
            .await
            .expect("stop");

    assert_eq!(stopped.id, id);
    assert!(stopped.ended_at.is_some(), "ended_at should be set");
    let dur = stopped.duration_ms.expect("duration_ms set");
    assert!(dur >= 50, "expected >=50ms, got {dur}");

    // Round-trip via storage.
    let row = storage.get_meeting(&id).unwrap();
    assert!(row.ended_at.is_some());
    assert!(row.duration_ms.unwrap() >= 50);

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_stop_unknown_id_is_input_not_found() {
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
    let err = ClientT::request::<Meeting, _>(
        &client,
        "ipc.meeting.stop",
        rpc_params![json!({"id":"nope"})],
    )
    .await
    .expect_err("unknown id must error");

    let ClientError::Call(call_err) = err else {
        panic!("expected Call error, got {err:?}");
    };
    let data: serde_json::Value =
        serde_json::from_str(call_err.data().unwrap().get()).expect("error data is JSON");
    assert_eq!(
        data["kind"], "input_not_found",
        "expected InputNotFound kind, got {data}"
    );

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
