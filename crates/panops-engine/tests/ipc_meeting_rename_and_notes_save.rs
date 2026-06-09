//! Stage 1 of the editing-save slice: `ipc.meeting.rename` and
//! `ipc.notes.save` round-trip over the real UDS+WebSocket transport.
//!
//! Two tests:
//!   1. `meeting.rename` updates the row; `meeting.get` and
//!      `meeting.list` both reflect the new title.
//!   2. `notes.save` writes `<meeting_dir>/notes.md` with the
//!      supplied markdown AND replaces the meeting's `note` row with
//!      a single fresh row carrying the new content.
//!
//! Exercises the full path: jsonrpsee client -> UDS -> engine handler
//! -> spawn_blocking -> Storage + FS -> back out.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::{ClientT, Error as ClientError};
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownRegionsFake, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::Meeting;
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

fn services(
    storage: Arc<dyn panops_core::storage::Storage>,
    data_dir: std::path::PathBuf,
) -> EngineServices {
    EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage,
        data_dir,
        Arc::new(TranscriptFileFake::default()),
        Arc::new(KnownTurnsFake),
        Arc::new(FakeNotesExporter),
        Arc::new(KnownRegionsFake::default()),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_rename_updates_row_and_reflects_in_get_and_list() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let server = tokio::spawn({
        let socket = socket.clone();
        let services = services(storage.clone(), data_dir);
        let shutdown_rx = shutdown_rx.clone();
        async move {
            run_serve_in_process(&socket, services, Some(shutdown_rx))
                .await
                .unwrap();
        }
    });
    wait_for_socket(&socket).await;

    let client = uds_ws_client(&socket).await;
    let id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Old Title","language":"en"})],
    )
    .await
    .expect("meeting.start");

    // meeting.rename returns the updated Meeting shape.
    let renamed: Meeting = ClientT::request(
        &client,
        "ipc.meeting.rename",
        rpc_params![json!({"meeting_id":id,"title":"New Title"})],
    )
    .await
    .expect("meeting.rename");
    assert_eq!(renamed.id, id);
    assert_eq!(renamed.title, "New Title");
    assert_eq!(renamed.language, "en", "other fields unchanged");

    // meeting.get reflects the new title.
    let got: Meeting = ClientT::request(&client, "ipc.meeting.get", rpc_params![json!({"id":id})])
        .await
        .expect("meeting.get");
    assert_eq!(got.title, "New Title");

    // meeting.list reflects the new title in the summary row.
    let list: Vec<panops_protocol::MeetingSummary> =
        ClientT::request(&client, "ipc.meeting.list", rpc_params![json!({})])
            .await
            .expect("meeting.list");
    let ours = list.iter().find(|m| m.id == id).expect("row in list");
    assert_eq!(ours.title, "New Title");

    // Storage-level confirmation: the row persisted.
    let row = storage.get_meeting(&id).expect("row in storage");
    assert_eq!(row.title, "New Title");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_rename_unknown_id_is_input_not_found() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let server = tokio::spawn({
        let socket = socket.clone();
        let services = services(storage, data_dir);
        let shutdown_rx = shutdown_rx.clone();
        async move {
            run_serve_in_process(&socket, services, Some(shutdown_rx))
                .await
                .unwrap();
        }
    });
    wait_for_socket(&socket).await;

    let client = uds_ws_client(&socket).await;
    let err = ClientT::request::<Meeting, _>(
        &client,
        "ipc.meeting.rename",
        rpc_params![json!({"meeting_id":"nope","title":"X"})],
    )
    .await
    .expect_err("rename of unknown id must error");

    let ClientError::Call(call_err) = err else {
        panic!("expected Call error, got {err:?}");
    };
    let data: serde_json::Value =
        serde_json::from_str(call_err.data().unwrap().get()).expect("data is JSON");
    assert_eq!(data["kind"], "input_not_found");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notes_save_writes_file_and_replaces_note_row() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let server = tokio::spawn({
        let socket = socket.clone();
        let services = services(storage.clone(), data_dir.clone());
        let shutdown_rx = shutdown_rx.clone();
        async move {
            run_serve_in_process(&socket, services, Some(shutdown_rx))
                .await
                .unwrap();
        }
    });
    wait_for_socket(&socket).await;

    let client = uds_ws_client(&socket).await;
    let id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Notes Save Test","language":"en"})],
    )
    .await
    .expect("meeting.start");

    let markdown = "# Hand-written\n\nUser edited these notes.";
    let _: () = ClientT::request(
        &client,
        "ipc.notes.save",
        rpc_params![json!({"meeting_id":id,"markdown":markdown})],
    )
    .await
    .expect("notes.save");

    // 1. The on-disk file contains the markdown.
    let meeting_dir = data_dir.join("meetings").join(&id);
    let notes_file = meeting_dir.join("notes.md");
    assert!(notes_file.exists(), "notes.md must exist at {notes_file:?}");
    let on_disk = std::fs::read_to_string(&notes_file).expect("read notes.md");
    assert_eq!(on_disk, markdown);
    // The partial-file sibling must be gone (rename succeeded).
    assert!(
        !meeting_dir.join("notes.md.partial").exists(),
        "notes.md.partial must be cleaned up by the rename"
    );

    // 2. The storage row is replaced (single row, new content).
    let notes = storage
        .list_notes_for_meeting(&id)
        .expect("list notes after save");
    assert_eq!(notes.len(), 1, "replace must leave exactly one note row");
    assert_eq!(notes[0].content_md, markdown);
    assert_eq!(notes[0].dialect, "basic");
    assert_eq!(
        notes[0].primary_path,
        notes_file.to_string_lossy(),
        "primary_path must point at the written notes.md"
    );

    // 3. A second save replaces (does not append).
    let revised = "# Revised\n\nUser revised.";
    let _: () = ClientT::request(
        &client,
        "ipc.notes.save",
        rpc_params![json!({"meeting_id":id,"markdown":revised})],
    )
    .await
    .expect("second notes.save");
    let notes = storage.list_notes_for_meeting(&id).unwrap();
    assert_eq!(notes.len(), 1, "second save must still leave one row");
    assert_eq!(notes[0].content_md, revised);
    let on_disk = std::fs::read_to_string(&notes_file).unwrap();
    assert_eq!(on_disk, revised);

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notes_save_unknown_meeting_is_input_not_found() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let server = tokio::spawn({
        let socket = socket.clone();
        let services = services(storage, data_dir);
        let shutdown_rx = shutdown_rx.clone();
        async move {
            run_serve_in_process(&socket, services, Some(shutdown_rx))
                .await
                .unwrap();
        }
    });
    wait_for_socket(&socket).await;

    let client = uds_ws_client(&socket).await;
    let err = ClientT::request::<(), _>(
        &client,
        "ipc.notes.save",
        rpc_params![json!({"meeting_id":"nope","markdown":"x"})],
    )
    .await
    .expect_err("notes.save of unknown meeting must error");

    let ClientError::Call(call_err) = err else {
        panic!("expected Call error, got {err:?}");
    };
    let data: serde_json::Value =
        serde_json::from_str(call_err.data().unwrap().get()).expect("data is JSON");
    assert_eq!(data["kind"], "input_not_found");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
