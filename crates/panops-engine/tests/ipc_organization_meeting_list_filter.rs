//! Phase B PR B2 — organization IPC exposes spaces and meeting.list filters.
//! Creates two meetings, assigns one to a space, then verifies the space_id
//! filter returns only the assigned meeting over the socket API.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownRegionsFake, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::{MeetingSummary, Space};
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_list_space_filter_returns_only_assigned_meeting() {
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

    let assigned_meeting_id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Assigned"})],
    )
    .await
    .expect("meeting.start assigned");
    let _unassigned_meeting_id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Unassigned"})],
    )
    .await
    .expect("meeting.start unassigned");

    let space: Space = ClientT::request(
        &client,
        "ipc.space.create",
        rpc_params![json!({"name":"Work"})],
    )
    .await
    .expect("space.create");

    let _: () = ClientT::request(
        &client,
        "ipc.meeting.assign",
        rpc_params![
            json!({"meeting_id": assigned_meeting_id.clone(), "space_id": space.id.clone()})
        ],
    )
    .await
    .expect("meeting.assign");

    let filtered: Vec<MeetingSummary> = ClientT::request(
        &client,
        "ipc.meeting.list",
        rpc_params![json!({"space_id": space.id.clone()})],
    )
    .await
    .expect("meeting.list filtered by space_id");

    assert_eq!(
        filtered.len(),
        1,
        "expected only assigned row: {filtered:?}"
    );
    assert_eq!(filtered[0].id, assigned_meeting_id);
    assert_eq!(filtered[0].title, "Assigned");
    assert_eq!(filtered[0].space_id.as_deref(), Some(space.id.as_str()));
    assert!(filtered[0].project_id.is_none());

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
