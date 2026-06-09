//! Slice 05 — `notes.generate` round-trips through UDS+WS, completes
//! the pipeline on the blocking pool, and surfaces `Event::JobDone`
//! on `events.subscribe`.
//!
//! Deterministic ASR / diar / MockLlm wiring lives in
//! `tests/common/notes_pipeline.rs` (shared with the slice-06
//! `ipc_notes_generate_*` tests so we don't carry parallel copies of
//! the golden segments + prompt fingerprints).

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use jsonrpsee::core::client::{ClientT, Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use panops_engine::server::run_serve_in_process;
use panops_protocol::{Event, JobAccepted};
use tempfile::tempdir;
use tokio::sync::watch;

use common::notes_pipeline::build_deterministic_notes_services;
use common::{tempdir_storage, uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notes_generate_round_trip_emits_job_done() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let audio_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-engine")
        .join("tests/fixtures/audio");
    let audio_path = audio_dir.join("multi_speaker_60s.wav");

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = build_deterministic_notes_services(storage, data_dir);

    let server_socket = socket.clone();
    let server_shutdown = shutdown_rx.clone();
    let server = tokio::spawn(async move {
        run_serve_in_process(&server_socket, services, Some(server_shutdown))
            .await
            .unwrap();
    });

    wait_for_socket(&socket).await;

    let client = uds_ws_client(&socket).await;

    // Subscribe FIRST so we don't race the job-completion broadcast.
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
            "audio": audio_path.to_string_lossy(),
            "dialect": "basic",
        })],
    )
    .await
    .expect("call notes.generate");

    let primary_file = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let event = subscription
                .next()
                .await
                .expect("subscription open")
                .expect("payload deserialises");
            match event {
                Event::JobDone(d) => return PathBuf::from(d.result.primary_file),
                Event::JobError(e) => panic!("expected JobDone, got JobError: {:?}", e.error),
                Event::Unknown(v) => panic!("expected JobDone, got Unknown: {v}"),
                // Slice 11 adds Screenshot and RecordingProgress events; ignore them
                // in this notes pipeline test (they may arrive from concurrent tests).
                Event::JobProgress(_) => {
                    continue;
                }
            }
        }
    })
    .await
    .expect("event arrived within 60s");
    assert!(
        primary_file.exists(),
        "primary_file does not exist: {primary_file:?}"
    );

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
