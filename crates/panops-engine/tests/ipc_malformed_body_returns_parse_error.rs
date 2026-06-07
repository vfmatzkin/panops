//! #88 — a malformed JSON-RPC body returns code -32700 (parse error) over
//! the wire, instead of dropping the connection or panicking the server.
//!
//! The slice-05 harness connects with jsonrpsee's high-level `WsClient`,
//! which serializes every outgoing request and so can't transmit a broken
//! body. This test drives a raw WebSocket (tokio-tungstenite) over the same
//! UDS to push a deliberately malformed frame through the server's JSON-RPC
//! parser. JSON-RPC 2.0 §5.1 fixes the parse-error code at -32700.

mod common;

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownRegionsFake, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use tempfile::tempdir;
use tokio::net::UnixStream;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

use common::{tempdir_storage, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_json_rpc_body_returns_parse_error() {
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

    // Raw WS over the UDS so we can send a body jsonrpsee's `WsClient` would
    // refuse to serialize. The `ws://localhost` URL is a placeholder — the
    // handshake rides the pre-connected UnixStream, mirroring `uds_ws_client`.
    let stream = UnixStream::connect(&socket).await.expect("connect uds");
    let (mut ws, _resp) = tokio_tungstenite::client_async("ws://localhost/", stream)
        .await
        .expect("ws handshake");

    // Mismatched braces: valid UTF-8, invalid JSON.
    ws.send(Message::text("{ \"broken json"))
        .await
        .expect("send malformed frame");

    // The connection must stay open and answer with a JSON-RPC error frame.
    // Skip any control frames (the server has ping disabled, but be defensive)
    // and fail loudly if the server drops the connection instead of replying.
    let envelope = loop {
        let frame = ws
            .next()
            .await
            .expect("server must answer, not drop the connection")
            .expect("ws frame is ok");
        match frame {
            Message::Text(text) => {
                break serde_json::from_str::<serde_json::Value>(text.as_str())
                    .expect("response body is valid JSON");
            }
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("expected a JSON-RPC error frame, got: {other:?}"),
        }
    };

    assert_eq!(
        envelope["error"]["code"], -32700,
        "expected JSON-RPC parse error (-32700), got: {envelope}"
    );

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
