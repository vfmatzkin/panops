//! Slice 06 — `notes.generate` with an existing `meeting_id` attaches
//! the generated note to that meeting and writes into its `dir_path`.
//! No new meeting is auto-created.

mod common;

use std::path::Path;
use std::sync::Arc;
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
async fn notes_generate_with_existing_meeting_id_attaches_note() {
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

    // Create meeting up front via meeting.start.
    let meeting_id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![serde_json::json!({"title":"My Meeting"})],
    )
    .await
    .expect("meeting.start");

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
            "meeting_id": meeting_id.clone(),
        })],
    )
    .await
    .expect("notes.generate");

    let result_meeting_id = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let event = subscription
                .next()
                .await
                .expect("subscription open")
                .expect("payload deserialises");
            match event {
                Event::JobDone(d) => return d.result.meeting_id,
                Event::JobError(e) => panic!("expected JobDone, got JobError: {:?}", e.error),
                Event::Unknown(v) => panic!("expected JobDone, got Unknown: {v}"),
                // Slice 11 adds Screenshot and RecordingProgress events; ignore them
                // in this notes pipeline test (they may arrive from concurrent tests).
                Event::Screenshot(_) | Event::RecordingProgress(_) => continue,
            }
        }
    })
    .await
    .expect("event within 60s");

    assert_eq!(
        result_meeting_id, meeting_id,
        "result.meeting_id should equal the supplied meeting_id"
    );

    // Note row attached to the supplied meeting.
    let notes = storage.list_notes_for_meeting(&meeting_id).unwrap();
    assert_eq!(notes.len(), 1, "exactly one note attached");
    assert_eq!(notes[0].meeting_id, meeting_id);

    // No second meeting was auto-created.
    let list = storage.list_meetings().unwrap();
    assert_eq!(list.len(), 1, "expected exactly one meeting");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
