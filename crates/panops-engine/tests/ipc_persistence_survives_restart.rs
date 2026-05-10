//! Slice 06 — meetings created in one server lifecycle are visible
//! in the next. Uses `RusqliteStorage` (not the in-memory fake) so
//! the on-disk DB actually has to persist.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownRegionsFake, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_core::storage::Storage;
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_portable::rusqlite_storage::RusqliteStorage;
use panops_protocol::MeetingSummary;
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::watch;

use common::{uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meetings_created_in_one_session_visible_in_next() {
    let data_tmp = tempdir().unwrap();
    let data_dir = data_tmp.path().to_owned();
    let socket_tmp = tempdir().unwrap();
    let socket = socket_tmp.path().join("engine.sock");

    // === Session A ===
    {
        let storage: Arc<dyn Storage> =
            Arc::new(RusqliteStorage::new(&data_dir.join("panops.db")).unwrap());
        let services = EngineServices::ready(
            Arc::new(MockLlm::default()),
            storage,
            data_dir.clone(),
            Arc::new(TranscriptFileFake::default()),
            Arc::new(KnownTurnsFake),
            Arc::new(FakeNotesExporter),
            Arc::new(KnownRegionsFake::new()),
        );

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server_socket = socket.clone();
        let server = tokio::spawn(async move {
            run_serve_in_process(&server_socket, services, Some(shutdown_rx))
                .await
                .unwrap();
        });
        wait_for_socket(&socket).await;

        let client = uds_ws_client(&socket).await;
        let _id: String = ClientT::request(
            &client,
            "ipc.meeting.start",
            rpc_params![json!({"title":"Session A"})],
        )
        .await
        .expect("session A start");

        // Drop the client BEFORE shutdown so the WS task can finish
        // cleanly; otherwise the server may wait on the still-open
        // connection beyond shutdown.
        drop(client);

        let _ = shutdown_tx.send(true);
        let _ = server.await;
    }
    // RusqliteStorage Arc dropped here; on-disk DB closed.

    // === Session B ===
    {
        let storage: Arc<dyn Storage> = Arc::new(
            RusqliteStorage::new(&data_dir.join("panops.db"))
                .expect("re-open should succeed against existing DB at version 1"),
        );
        let services = EngineServices::ready(
            Arc::new(MockLlm::default()),
            storage,
            data_dir.clone(),
            Arc::new(TranscriptFileFake::default()),
            Arc::new(KnownTurnsFake),
            Arc::new(FakeNotesExporter),
            Arc::new(KnownRegionsFake::new()),
        );

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server_socket = socket.clone();
        let server = tokio::spawn(async move {
            run_serve_in_process(&server_socket, services, Some(shutdown_rx))
                .await
                .unwrap();
        });
        wait_for_socket(&socket).await;

        let client = uds_ws_client(&socket).await;
        let rows: Vec<MeetingSummary> =
            ClientT::request(&client, "ipc.meeting.list", rpc_params![])
                .await
                .expect("session B list");

        assert_eq!(
            rows.len(),
            1,
            "expected the meeting from session A to persist; got {rows:?}"
        );
        assert_eq!(rows[0].title, "Session A");

        drop(client);
        let _ = shutdown_tx.send(true);
        let _ = server.await;
    }
}
