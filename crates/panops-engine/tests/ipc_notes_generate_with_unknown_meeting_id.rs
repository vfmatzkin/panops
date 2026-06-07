//! Slice 06 — `notes.generate` with a `meeting_id` that doesn't exist
//! surfaces an `InputNotFound` (the meeting lookup is the first thing
//! the pipeline does after canonicalize, so the error arrives via
//! `job.error` over the events subscription).

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::{ClientT, Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use panops_engine::server::run_serve_in_process;
use panops_protocol::{Event, IpcError, JobAccepted};
use tempfile::tempdir;
use tokio::sync::watch;

use common::notes_pipeline::build_deterministic_notes_services;
use common::{tempdir_storage, uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notes_generate_with_unknown_meeting_id_returns_input_not_found() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let audio_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-engine")
        .join("tests/fixtures/audio/multi_speaker_60s.wav");

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = build_deterministic_notes_services(Arc::clone(&storage), data_dir);

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
    .expect("subscribe");

    let _accepted: JobAccepted = ClientT::request(
        &client,
        "ipc.notes.generate",
        rpc_params![serde_json::json!({
            "audio": audio_path.to_string_lossy(),
            "dialect": "basic",
            "meeting_id": "unknown-meeting-id",
        })],
    )
    .await
    .expect("notes.generate accepts the request even for unknown id");

    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let event = subscription
                .next()
                .await
                .expect("subscription open")
                .expect("payload deserialises");
            match event {
                Event::JobError(e) => match e.error {
                    IpcError::InputNotFound { path } => {
                        assert!(
                            path.contains("meeting"),
                            "expected meeting in path, got: {path}"
                        );
                        return;
                    }
                    other => panic!("expected InputNotFound, got {other:?}"),
                },
                Event::JobDone(d) => panic!("expected JobError, got JobDone: {:?}", d.result),
                Event::Unknown(v) => panic!("expected JobError, got Unknown: {v}"),
                // Slice 11 adds Screenshot and RecordingProgress events; ignore them
                // in this notes pipeline test (they may arrive from concurrent tests).
                Event::Screenshot(_) | Event::RecordingProgress(_) | Event::JobProgress(_) => {
                    continue;
                }
            }
        }
    })
    .await
    .expect("event within 60s");

    // No meeting was created (the lookup failed before any side effects).
    let list = storage.list_meetings().unwrap();
    assert!(list.is_empty(), "no meetings should exist, got {list:?}");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
