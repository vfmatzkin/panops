//! Slice 05 — calling an unknown method returns JSON-RPC error code -32601.

use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use tempfile::tempdir;
use tokio::net::UnixStream;
use tokio::sync::Notify;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_method_returns_method_not_found() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let shutdown = Arc::new(Notify::new());

    let services = EngineServices {
        llm: Arc::new(MockLlm::default()),
        asr: Arc::new(TranscriptFileFake),
        diar: Arc::new(KnownTurnsFake),
        exporter: Arc::new(FakeNotesExporter),
    };

    let server_socket = socket.clone();
    let server_shutdown = shutdown.clone();
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

    shutdown.notify_waiters();
    let _ = server.await;
}

async fn wait_for_socket(path: &std::path::Path) {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if path.exists() && UnixStream::connect(path).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("socket never became connectable: {path:?}");
}

async fn uds_ws_client(path: &std::path::Path) -> jsonrpsee::ws_client::WsClient {
    let stream = UnixStream::connect(path).await.expect("connect uds");
    jsonrpsee::ws_client::WsClientBuilder::default()
        .build_with_stream("ws://localhost", stream)
        .await
        .expect("build ws client over uds")
}
