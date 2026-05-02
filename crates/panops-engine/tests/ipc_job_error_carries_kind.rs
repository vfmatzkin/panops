//! Slice 05 — `notes.generate` against a missing audio path emits
//! `Event::JobError` with `IpcError::InputNotFound` (kind tag
//! `"input_not_found"` on the wire).
//!
//! Uses `TranscriptFileFake` from `panops-core::conformance::fakes` —
//! it returns `AsrError::AudioNotFound` for nonexistent paths, and
//! `panops-protocol`'s `domain-conversions` feature maps that to
//! `IpcError::InputNotFound`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::{ClientT, Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::{Event, IpcError, JobAccepted};
use tempfile::tempdir;
use tokio::sync::watch;

use common::{uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notes_generate_emits_job_error_with_input_not_found_kind() {
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

    let mut subscription: Subscription<Event> = SubscriptionClientT::subscribe(
        &client,
        "ipc.events.subscribe",
        rpc_params![],
        "ipc.events.unsubscribe",
    )
    .await
    .expect("subscribe to events");

    let _accepted: JobAccepted = ClientT::request(
        &client,
        "ipc.notes.generate",
        rpc_params![serde_json::json!({
            "audio": "/nonexistent/path.wav",
        })],
    )
    .await
    .expect("call notes.generate");

    let event = tokio::time::timeout(Duration::from_secs(10), subscription.next())
        .await
        .expect("event arrived within 10s")
        .expect("subscription not closed")
        .expect("event payload deserialised");

    match event {
        Event::JobError(err) => {
            assert!(
                matches!(err.error, IpcError::InputNotFound { .. }),
                "expected InputNotFound, got {:?}",
                err.error
            );
            // Wire-level check: serialised payload carries `kind: "input_not_found"`.
            let json = serde_json::to_value(&err.error).unwrap();
            assert_eq!(
                json.get("kind").and_then(|v| v.as_str()),
                Some("input_not_found")
            );
        }
        Event::JobDone(d) => panic!("expected JobError, got JobDone: {:?}", d),
        Event::Unknown(v) => panic!("expected JobError, got Unknown: {v}"),
    }

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
