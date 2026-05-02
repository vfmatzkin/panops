//! Slice 05 — calling an unknown method returns JSON-RPC error code -32601.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use tempfile::tempdir;
use tokio::sync::watch;

use common::{uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_method_returns_method_not_found() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
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
    let err = ClientT::request::<serde_json::Value, _>(&client, "ipc.foo.bar", rpc_params![])
        .await
        .expect_err("foo.bar must error");

    let msg = format!("{err:?}");
    assert!(
        msg.contains("Method not found")
            || msg.contains("-32601")
            || msg.contains("MethodNotFound"),
        "expected method-not-found error, got: {msg}"
    );

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
