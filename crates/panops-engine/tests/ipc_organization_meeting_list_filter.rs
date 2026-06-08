//! Phase B PR B2 — organization IPC exposes spaces and meeting.list filters.
//! Creates two meetings, assigns one to a space, then verifies the space_id
//! filter returns only the assigned meeting over the socket API.

mod common;

use std::sync::Arc;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownRegionsFake, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_core::storage::Storage;
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::{
    MeetingSummary, Project, ProjectListResult, Space, SpaceListResult, Tag, TagListResult,
};
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

struct IpcHarness {
    _socket_tmp: tempfile::TempDir,
    _storage_tmp: tempfile::TempDir,
    storage: Arc<dyn Storage>,
    client: jsonrpsee::ws_client::WsClient,
    shutdown_tx: watch::Sender<bool>,
    server: JoinHandle<()>,
}

async fn start_harness() -> IpcHarness {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage.clone(),
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

    let client = uds_ws_client(&socket).await;

    IpcHarness {
        _socket_tmp: dir,
        _storage_tmp: storage_tmp,
        storage,
        client,
        shutdown_tx,
        server,
    }
}

async fn shutdown(harness: IpcHarness) {
    let _ = harness.shutdown_tx.send(true);
    let _ = harness.server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_list_space_filter_returns_only_assigned_meeting() {
    let harness = start_harness().await;
    let client = &harness.client;

    let assigned_meeting_id: String = ClientT::request(
        client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Assigned"})],
    )
    .await
    .expect("meeting.start assigned");
    let _unassigned_meeting_id: String = ClientT::request(
        client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Unassigned"})],
    )
    .await
    .expect("meeting.start unassigned");

    let space: Space = ClientT::request(
        client,
        "ipc.space.create",
        rpc_params![json!({"name":"Work"})],
    )
    .await
    .expect("space.create");

    let _: () = ClientT::request(
        client,
        "ipc.meeting.assign",
        rpc_params![
            json!({"meeting_id": assigned_meeting_id.clone(), "space_id": space.id.clone()})
        ],
    )
    .await
    .expect("meeting.assign");

    let filtered: Vec<MeetingSummary> = ClientT::request(
        client,
        "ipc.meeting.list",
        rpc_params![json!({"space_id": space.id.clone()})],
    )
    .await
    .expect("meeting.list filtered by space_id");

    assert_eq!(
        filtered.len(),
        1,
        "expected only assigned row: {filtered:?}"
    );
    assert_eq!(filtered[0].id, assigned_meeting_id);
    assert_eq!(filtered[0].title, "Assigned");
    assert_eq!(filtered[0].space_id.as_deref(), Some(space.id.as_str()));
    assert!(filtered[0].project_id.is_none());

    shutdown(harness).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn space_rename_and_delete_are_reflected_by_space_list() {
    let harness = start_harness().await;
    let client = &harness.client;

    let space: Space = ClientT::request(
        client,
        "ipc.space.create",
        rpc_params![json!({"name":"Work"})],
    )
    .await
    .expect("space.create");
    assert_eq!(space.id, "space_1");
    assert_eq!(space.name, "Work");
    assert_eq!(space.position, 0);

    let _: () = ClientT::request(
        client,
        "ipc.space.rename",
        rpc_params![json!({"id": space.id.clone(), "name":"Renamed Work"})],
    )
    .await
    .expect("space.rename");

    let listed: SpaceListResult = ClientT::request(client, "ipc.space.list", rpc_params![])
        .await
        .expect("space.list after rename");
    assert_eq!(
        listed.spaces,
        vec![Space {
            id: space.id.clone(),
            name: "Renamed Work".into(),
            position: 0,
        }]
    );

    let _: () = ClientT::request(
        client,
        "ipc.space.delete",
        rpc_params![json!({"id": space.id.clone()})],
    )
    .await
    .expect("space.delete");

    let listed: SpaceListResult = ClientT::request(client, "ipc.space.list", rpc_params![])
        .await
        .expect("space.list after delete");
    assert!(listed.spaces.is_empty(), "space should be gone: {listed:?}");

    shutdown(harness).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_create_rename_delete_are_reflected_by_project_list_space_filter() {
    let harness = start_harness().await;
    let client = &harness.client;

    let target_space: Space = ClientT::request(
        client,
        "ipc.space.create",
        rpc_params![json!({"name":"Client Work"})],
    )
    .await
    .expect("target space.create");
    let other_space: Space = ClientT::request(
        client,
        "ipc.space.create",
        rpc_params![json!({"name":"Personal"})],
    )
    .await
    .expect("other space.create");
    assert_eq!(target_space.id, "space_1");
    assert_eq!(other_space.id, "space_2");

    let project: Project = ClientT::request(
        client,
        "ipc.project.create",
        rpc_params![json!({"space_id": target_space.id.clone(), "name":"Launch"})],
    )
    .await
    .expect("project.create");
    let other_project: Project = ClientT::request(
        client,
        "ipc.project.create",
        rpc_params![json!({"space_id": other_space.id.clone(), "name":"Errands"})],
    )
    .await
    .expect("other project.create");
    assert_eq!(
        project,
        Project {
            id: "project_1".into(),
            space_id: target_space.id.clone(),
            name: "Launch".into(),
            position: 0,
        }
    );
    assert_eq!(other_project.id, "project_2");

    let listed: ProjectListResult = ClientT::request(
        client,
        "ipc.project.list",
        rpc_params![json!({"space_id": target_space.id.clone()})],
    )
    .await
    .expect("project.list target space");
    assert_eq!(listed.projects, vec![project.clone()]);

    let _: () = ClientT::request(
        client,
        "ipc.project.rename",
        rpc_params![json!({"id": project.id.clone(), "name":"Renamed Launch"})],
    )
    .await
    .expect("project.rename");

    let listed: ProjectListResult = ClientT::request(
        client,
        "ipc.project.list",
        rpc_params![json!({"space_id": target_space.id.clone()})],
    )
    .await
    .expect("project.list after rename");
    assert_eq!(
        listed.projects,
        vec![Project {
            id: project.id.clone(),
            space_id: target_space.id.clone(),
            name: "Renamed Launch".into(),
            position: 0,
        }]
    );

    let _: () = ClientT::request(
        client,
        "ipc.project.delete",
        rpc_params![json!({"id": project.id.clone()})],
    )
    .await
    .expect("project.delete");

    let listed: ProjectListResult = ClientT::request(
        client,
        "ipc.project.list",
        rpc_params![json!({"space_id": target_space.id.clone()})],
    )
    .await
    .expect("project.list after delete");
    assert!(
        listed.projects.is_empty(),
        "deleted project should be gone from target space: {listed:?}"
    );
    let other_listed: ProjectListResult = ClientT::request(
        client,
        "ipc.project.list",
        rpc_params![json!({"space_id": other_space.id.clone()})],
    )
    .await
    .expect("project.list other space after target delete");
    assert_eq!(other_listed.projects, vec![other_project]);

    shutdown(harness).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tag_create_is_idempotent_and_assign_unassign_updates_meeting_tags() {
    let harness = start_harness().await;
    let client = &harness.client;

    let meeting_id: String = ClientT::request(
        client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Tagged"})],
    )
    .await
    .expect("meeting.start tagged");

    let tag: Tag = ClientT::request(
        client,
        "ipc.tag.create",
        rpc_params![json!({"name":"customer"})],
    )
    .await
    .expect("tag.create");
    let same_tag: Tag = ClientT::request(
        client,
        "ipc.tag.create",
        rpc_params![json!({"name":"customer"})],
    )
    .await
    .expect("tag.create duplicate");
    assert_eq!(
        tag,
        Tag {
            id: "tag_1".into(),
            name: "customer".into(),
        }
    );
    assert_eq!(same_tag, tag, "duplicate tag.create should be idempotent");

    let listed: TagListResult = ClientT::request(client, "ipc.tag.list", rpc_params![])
        .await
        .expect("tag.list");
    assert_eq!(listed.tags, vec![tag.clone()]);

    let _: () = ClientT::request(
        client,
        "ipc.tag.assign",
        rpc_params![json!({"meeting_id": meeting_id.clone(), "tag_id": tag.id.clone()})],
    )
    .await
    .expect("tag.assign");

    let meetings: Vec<MeetingSummary> = ClientT::request(client, "ipc.meeting.list", rpc_params![])
        .await
        .expect("meeting.list after tag.assign");
    assert_eq!(meetings.len(), 1, "expected one meeting: {meetings:?}");
    assert_eq!(meetings[0].id, meeting_id);
    assert_eq!(meetings[0].title, "Tagged");
    assert_eq!(meetings[0].tags, vec![tag.id.clone()]);

    let meeting_tags = harness
        .storage
        .list_tags_for_meeting(&meeting_id)
        .expect("storage list_tags_for_meeting after tag.assign");
    assert_eq!(
        meeting_tags,
        vec![panops_core::storage::Tag::from(tag.clone())]
    );

    let _: () = ClientT::request(
        client,
        "ipc.tag.unassign",
        rpc_params![json!({"meeting_id": meeting_id.clone(), "tag_id": tag.id.clone()})],
    )
    .await
    .expect("tag.unassign");

    let meetings: Vec<MeetingSummary> = ClientT::request(client, "ipc.meeting.list", rpc_params![])
        .await
        .expect("meeting.list after tag.unassign");
    assert_eq!(meetings.len(), 1, "expected one meeting: {meetings:?}");
    assert_eq!(meetings[0].id, meeting_id);
    assert!(
        meetings[0].tags.is_empty(),
        "tag should be gone: {meetings:?}"
    );
    let meeting_tags = harness
        .storage
        .list_tags_for_meeting(&meeting_id)
        .expect("storage list_tags_for_meeting after tag.unassign");
    assert!(
        meeting_tags.is_empty(),
        "tag should be gone: {meeting_tags:?}"
    );

    let _: () = ClientT::request(
        client,
        "ipc.tag.delete",
        rpc_params![json!({"id": tag.id.clone()})],
    )
    .await
    .expect("tag.delete");

    let listed: TagListResult = ClientT::request(client, "ipc.tag.list", rpc_params![])
        .await
        .expect("tag.list after delete");
    assert!(
        listed.tags.is_empty(),
        "deleted tag should be gone: {listed:?}"
    );

    shutdown(harness).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_assign_with_project_id_sets_project_and_project_space() {
    let harness = start_harness().await;
    let client = &harness.client;

    let meeting_id: String = ClientT::request(
        client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Project Meeting"})],
    )
    .await
    .expect("meeting.start");
    let space: Space = ClientT::request(
        client,
        "ipc.space.create",
        rpc_params![json!({"name":"Work"})],
    )
    .await
    .expect("space.create");
    let project: Project = ClientT::request(
        client,
        "ipc.project.create",
        rpc_params![json!({"space_id": space.id.clone(), "name":"Launch"})],
    )
    .await
    .expect("project.create");

    let _: () = ClientT::request(
        client,
        "ipc.meeting.assign",
        rpc_params![json!({"meeting_id": meeting_id.clone(), "project_id": project.id.clone()})],
    )
    .await
    .expect("meeting.assign project");

    let listed: Vec<MeetingSummary> = ClientT::request(client, "ipc.meeting.list", rpc_params![])
        .await
        .expect("meeting.list after project assign");
    assert_eq!(listed.len(), 1, "expected one meeting: {listed:?}");
    assert_eq!(listed[0].id, meeting_id);
    assert_eq!(listed[0].title, "Project Meeting");
    assert_eq!(listed[0].project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(listed[0].space_id.as_deref(), Some(space.id.as_str()));

    shutdown(harness).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_list_project_tag_and_unsorted_filters_return_exact_meetings() {
    let harness = start_harness().await;
    let client = &harness.client;

    let project_meeting_id: String = ClientT::request(
        client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Project Filter Match"})],
    )
    .await
    .expect("meeting.start project match");
    let other_project_meeting_id: String = ClientT::request(
        client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Other Project"})],
    )
    .await
    .expect("meeting.start other project");
    let tagged_meeting_id: String = ClientT::request(
        client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Tagged Filter Match"})],
    )
    .await
    .expect("meeting.start tagged match");
    let unsorted_meeting_id: String = ClientT::request(
        client,
        "ipc.meeting.start",
        rpc_params![json!({"title":"Inbox"})],
    )
    .await
    .expect("meeting.start unsorted");

    let space: Space = ClientT::request(
        client,
        "ipc.space.create",
        rpc_params![json!({"name":"Work"})],
    )
    .await
    .expect("space.create work");
    let other_space: Space = ClientT::request(
        client,
        "ipc.space.create",
        rpc_params![json!({"name":"Other Space"})],
    )
    .await
    .expect("space.create other");
    let project: Project = ClientT::request(
        client,
        "ipc.project.create",
        rpc_params![json!({"space_id": space.id.clone(), "name":"Launch"})],
    )
    .await
    .expect("project.create launch");
    let other_project: Project = ClientT::request(
        client,
        "ipc.project.create",
        rpc_params![json!({"space_id": other_space.id.clone(), "name":"Ops"})],
    )
    .await
    .expect("project.create ops");
    let tag: Tag = ClientT::request(
        client,
        "ipc.tag.create",
        rpc_params![json!({"name":"customer"})],
    )
    .await
    .expect("tag.create customer");

    let _: () = ClientT::request(
        client,
        "ipc.meeting.assign",
        rpc_params![
            json!({"meeting_id": project_meeting_id.clone(), "project_id": project.id.clone()})
        ],
    )
    .await
    .expect("meeting.assign project match");
    let _: () = ClientT::request(
        client,
        "ipc.meeting.assign",
        rpc_params![
            json!({"meeting_id": other_project_meeting_id.clone(), "project_id": other_project.id.clone()})
        ],
    )
    .await
    .expect("meeting.assign other project");
    let _: () = ClientT::request(
        client,
        "ipc.meeting.assign",
        rpc_params![json!({"meeting_id": tagged_meeting_id.clone(), "space_id": space.id.clone()})],
    )
    .await
    .expect("meeting.assign tagged to a space");
    let _: () = ClientT::request(
        client,
        "ipc.tag.assign",
        rpc_params![json!({"meeting_id": tagged_meeting_id.clone(), "tag_id": tag.id.clone()})],
    )
    .await
    .expect("tag.assign tagged match");

    let project_filtered: Vec<MeetingSummary> = ClientT::request(
        client,
        "ipc.meeting.list",
        rpc_params![json!({"project_id": project.id.clone()})],
    )
    .await
    .expect("meeting.list project_id filter");
    assert_eq!(
        meeting_ids(&project_filtered),
        vec![project_meeting_id.clone()],
        "project filter should only return matching project: {project_filtered:?}"
    );
    assert_eq!(project_filtered[0].title, "Project Filter Match");
    assert_eq!(
        project_filtered[0].project_id.as_deref(),
        Some(project.id.as_str())
    );
    assert_eq!(
        project_filtered[0].space_id.as_deref(),
        Some(space.id.as_str())
    );

    let tag_filtered: Vec<MeetingSummary> = ClientT::request(
        client,
        "ipc.meeting.list",
        rpc_params![json!({"tag_id": tag.id.clone()})],
    )
    .await
    .expect("meeting.list tag_id filter");
    assert_eq!(
        meeting_ids(&tag_filtered),
        vec![tagged_meeting_id.clone()],
        "tag filter should only return tagged meeting: {tag_filtered:?}"
    );
    assert_eq!(tag_filtered[0].title, "Tagged Filter Match");
    assert_eq!(tag_filtered[0].tags, vec![tag.id.clone()]);

    let unsorted: Vec<MeetingSummary> = ClientT::request(
        client,
        "ipc.meeting.list",
        rpc_params![json!({"unsorted": true})],
    )
    .await
    .expect("meeting.list unsorted filter");
    assert_eq!(
        meeting_ids(&unsorted),
        vec![unsorted_meeting_id.clone()],
        "unsorted filter should only return meetings with no space: {unsorted:?}"
    );
    assert_eq!(unsorted[0].title, "Inbox");
    assert!(unsorted[0].space_id.is_none());
    assert!(unsorted[0].project_id.is_none());

    shutdown(harness).await;
}

fn meeting_ids(meetings: &[MeetingSummary]) -> Vec<String> {
    let mut ids: Vec<String> = meetings.iter().map(|m| m.id.clone()).collect();
    ids.sort();
    ids
}
