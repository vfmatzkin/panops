//! Slice 06 — `notes.generate` with no `meeting_id` auto-creates a
//! meeting in the registry and writes notes into the canonical
//! `<data_dir>/meetings/<id>/` layout.

mod common;

use std::path::PathBuf;
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
async fn notes_generate_without_meeting_id_auto_creates_one() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let audio_dir = tempdir().unwrap();
    let audio_path = audio_dir.path().join("multi_speaker_60s.wav");
    std::fs::write(&audio_path, b"placeholder").unwrap();

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = build_deterministic_notes_services(Arc::clone(&storage), data_dir.clone());

    let server_socket = socket.clone();
    let server_shutdown = shutdown_rx.clone();
    let server = tokio::spawn(async move {
        run_serve_in_process(&server_socket, services, Some(server_shutdown))
            .await
            .unwrap();
    });
    wait_for_socket(&socket).await;

    let client = uds_ws_client(&socket).await;

    // Subscribe FIRST so we don't race the broadcast.
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

    let event = tokio::time::timeout(Duration::from_secs(60), subscription.next())
        .await
        .expect("event arrived within 60s")
        .expect("subscription not closed")
        .expect("event payload deserialised");

    let (primary_file, meeting_id) = match event {
        Event::JobDone(d) => (PathBuf::from(d.result.primary_file), d.result.meeting_id),
        Event::JobError(e) => panic!("expected JobDone, got JobError: {:?}", e.error),
        Event::Unknown(v) => panic!("expected JobDone, got Unknown: {v}"),
    };

    assert!(
        primary_file.exists(),
        "primary_file does not exist: {primary_file:?}"
    );
    assert!(!meeting_id.is_empty(), "meeting_id should be set");

    // Verify meeting actually exists in storage and the registry has
    // exactly one row (the auto-created one).
    let m = storage.get_meeting(&meeting_id).expect("row exists");
    assert!(
        m.dir_path.contains("/meetings/"),
        "dir_path should be canonical: {}",
        m.dir_path
    );
    let list = storage.list_meetings().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, meeting_id);

    // Note row attached.
    let notes = storage.list_notes_for_meeting(&meeting_id).unwrap();
    assert_eq!(notes.len(), 1);

    // Output went into the canonical meeting dir.
    assert!(
        primary_file.starts_with(data_dir.join("meetings").join(&meeting_id)),
        "primary_file {primary_file:?} should live under canonical meeting dir"
    );

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
