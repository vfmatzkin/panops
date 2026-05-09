//! Slice 06 — `ipc.meeting.delete` removes the registry row, cascades
//! the `note` rows via FK, and removes the on-disk meeting directory.
//! Unknown id surfaces as `InputNotFound`.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::{ClientT, Error as ClientError};
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_core::storage::NoteDraft;
use panops_engine::server::{EngineServices, run_serve_in_process};
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_removes_row_dir_and_cascades_notes() {
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
    let id: String = ClientT::request(&client, "ipc.meeting.start", rpc_params![json!({})])
        .await
        .expect("start");

    // Insert a note via storage directly so we can verify cascade.
    storage
        .create_note(NoteDraft {
            id: "n1".into(),
            meeting_id: id.clone(),
            dialect: "basic".into(),
            content_md: "# x".into(),
            primary_path: "/tmp/x".into(),
        })
        .unwrap();

    let meeting_dir = data_dir.join("meetings").join(&id);
    assert!(meeting_dir.exists());

    let _: () = ClientT::request(
        &client,
        "ipc.meeting.delete",
        rpc_params![json!({"id":id.clone()})],
    )
    .await
    .expect("delete");

    // Row gone.
    assert!(storage.get_meeting(&id).is_err());
    // Notes gone (FK cascade in real impl; manual filter in fake).
    assert!(storage.list_notes_for_meeting(&id).unwrap().is_empty());
    // Directory gone.
    assert!(
        !meeting_dir.exists(),
        "dir should be gone, found: {meeting_dir:?}"
    );

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_unknown_id_is_input_not_found() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage,
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
    let err = ClientT::request::<(), _>(
        &client,
        "ipc.meeting.delete",
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
