//! jsonrpsee `#[rpc]` trait + impl for slice 05's two methods.
//!
//! `events.subscribe` is a server-push subscription multiplexing
//! `job.done` / `job.error` over a shared broadcast channel. Wave 4I
//! wires the trait + the events subscription scaffold; Wave 5K plugs
//! `notes.generate` into the broadcast channel.
//!
//! Method handlers return `Result<T, ErrorObjectOwned>`. The
//! `IpcError`-shaped `data` field is preserved at the wire level via
//! `ipc_error_to_obj`, matching the slice spec's "Error mapping at the
//! RPC boundary" section.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::ErrorObjectOwned;
use panops_core::capture::CaptureSession;
use panops_core::merge::merge_speaker_turns;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::input::{MeetingMetadata, NotesInput};
use panops_core::notes::ir::StructuredNotes;
use panops_core::notes::pipeline::NotesGenerator;
use panops_core::notes::raw_transcript::write_raw_transcript;
use panops_protocol::{
    CaptureWindowsParams, CaptureWindowsResult, Event, IpcError, JobAccepted, JobDoneEvent,
    JobErrorEvent, JobProgressEvent, Meeting, MeetingAssignParams, MeetingConfig,
    MeetingDeleteVideoParams, MeetingDeleteVideoResult, MeetingListParams, MeetingRenameParams,
    MeetingSummary, NotesDialect, NotesGenerateParams, NotesGenerateResult, NotesSaveParams,
    Project, ProjectCreateParams, ProjectDeleteParams, ProjectListParams, ProjectListResult,
    ProjectRenameParams, RecordingAccepted, RecordingStartParams, RecordingStopParams,
    RecordingStopped, ServerInfo, Space, SpaceCreateParams, SpaceDeleteParams, SpaceListResult,
    SpaceRenameParams, Tag, TagAssignParams, TagCreateParams, TagDeleteParams, TagListResult,
    WindowInfo,
};
use tokio::sync::broadcast;

/// Wrapper for `meeting.{stop,get,delete,set_language}` request params.
/// jsonrpsee's `#[rpc]` macro accepts strongly-typed params via a single
/// struct argument; we use distinct types per method so `serde_json` can
/// validate the shape and so the API surface is self-documenting.
#[derive(Debug, ::serde::Deserialize)]
pub struct MeetingIdParam {
    pub id: String,
}

#[derive(Debug, ::serde::Deserialize)]
pub struct MeetingSetLanguageParams {
    pub id: String,
    pub language: String,
}

#[rpc(server, namespace = "ipc", namespace_separator = ".")]
pub(super) trait Ipc {
    #[method(name = "notes.generate")]
    async fn notes_generate(
        &self,
        params: NotesGenerateParams,
    ) -> Result<JobAccepted, ErrorObjectOwned>;

    #[method(name = "notes.save")]
    async fn notes_save(&self, params: NotesSaveParams) -> Result<(), ErrorObjectOwned>;

    #[method(name = "meeting.list")]
    async fn meeting_list(
        &self,
        params: Option<MeetingListParams>,
    ) -> Result<Vec<MeetingSummary>, ErrorObjectOwned>;

    #[method(name = "server.info")]
    async fn server_info(&self) -> Result<ServerInfo, ErrorObjectOwned>;

    #[method(name = "meeting.start")]
    async fn meeting_start(&self, params: MeetingConfig) -> Result<String, ErrorObjectOwned>;

    #[method(name = "meeting.stop")]
    async fn meeting_stop(&self, params: MeetingIdParam) -> Result<Meeting, ErrorObjectOwned>;

    #[method(name = "meeting.get")]
    async fn meeting_get(&self, params: MeetingIdParam) -> Result<Meeting, ErrorObjectOwned>;

    #[method(name = "meeting.set_language")]
    async fn meeting_set_language(
        &self,
        params: MeetingSetLanguageParams,
    ) -> Result<Meeting, ErrorObjectOwned>;

    #[method(name = "meeting.rename")]
    async fn meeting_rename(
        &self,
        params: MeetingRenameParams,
    ) -> Result<Meeting, ErrorObjectOwned>;

    #[method(name = "meeting.delete")]
    async fn meeting_delete(&self, params: MeetingIdParam) -> Result<(), ErrorObjectOwned>;

    #[method(name = "meeting.assign")]
    async fn meeting_assign(&self, params: MeetingAssignParams) -> Result<(), ErrorObjectOwned>;

    #[method(name = "meeting.deleteVideo")]
    async fn meeting_delete_video(
        &self,
        params: MeetingDeleteVideoParams,
    ) -> Result<MeetingDeleteVideoResult, ErrorObjectOwned>;

    #[method(name = "space.create")]
    async fn space_create(&self, params: SpaceCreateParams) -> Result<Space, ErrorObjectOwned>;

    #[method(name = "space.list")]
    async fn space_list(&self) -> Result<SpaceListResult, ErrorObjectOwned>;

    #[method(name = "space.rename")]
    async fn space_rename(&self, params: SpaceRenameParams) -> Result<(), ErrorObjectOwned>;

    #[method(name = "space.delete")]
    async fn space_delete(&self, params: SpaceDeleteParams) -> Result<(), ErrorObjectOwned>;

    #[method(name = "project.create")]
    async fn project_create(
        &self,
        params: ProjectCreateParams,
    ) -> Result<Project, ErrorObjectOwned>;

    #[method(name = "project.list")]
    async fn project_list(
        &self,
        params: Option<ProjectListParams>,
    ) -> Result<ProjectListResult, ErrorObjectOwned>;

    #[method(name = "project.rename")]
    async fn project_rename(&self, params: ProjectRenameParams) -> Result<(), ErrorObjectOwned>;

    #[method(name = "project.delete")]
    async fn project_delete(&self, params: ProjectDeleteParams) -> Result<(), ErrorObjectOwned>;

    #[method(name = "tag.create")]
    async fn tag_create(&self, params: TagCreateParams) -> Result<Tag, ErrorObjectOwned>;

    #[method(name = "tag.list")]
    async fn tag_list(&self) -> Result<TagListResult, ErrorObjectOwned>;

    #[method(name = "tag.delete")]
    async fn tag_delete(&self, params: TagDeleteParams) -> Result<(), ErrorObjectOwned>;

    #[method(name = "tag.assign")]
    async fn tag_assign(&self, params: TagAssignParams) -> Result<(), ErrorObjectOwned>;

    #[method(name = "tag.unassign")]
    async fn tag_unassign(&self, params: TagAssignParams) -> Result<(), ErrorObjectOwned>;

    #[method(name = "recording.start")]
    async fn recording_start(
        &self,
        params: RecordingStartParams,
    ) -> Result<RecordingAccepted, ErrorObjectOwned>;

    #[method(name = "recording.stop")]
    async fn recording_stop(
        &self,
        params: RecordingStopParams,
    ) -> Result<RecordingStopped, ErrorObjectOwned>;

    /// Deprecated: superseded by the app-side `SCContentSharingPicker`, which now
    /// drives capture-source selection (window/display/app/region) directly in the
    /// Mac shell — it returns the live `SCContentFilter` the app previews from and
    /// the serializable target the recording starts against. Kept only as a
    /// fallback for clients without the native picker (e.g. headless callers); the
    /// Panops app no longer calls it.
    #[method(name = "capture.windows")]
    async fn capture_windows(
        &self,
        params: CaptureWindowsParams,
    ) -> Result<CaptureWindowsResult, ErrorObjectOwned>;

    #[subscription(
        name = "events.subscribe" => "events",
        unsubscribe = "events.unsubscribe",
        item = Event
    )]
    async fn subscribe_events(&self) -> SubscriptionResult;
}

pub(super) struct IpcImpl {
    pub(super) services: Arc<crate::server::EngineServices>,
    pub(super) events_tx: broadcast::Sender<Event>,
    /// Active capture sessions keyed by recording_id (meeting_id).
    /// Persisted from `recording.start` and looked up in `recording.stop`
    /// so the real session (with correct started_at_ms) is passed to
    /// `stop_capture` instead of a fabricated placeholder.
    /// Wrapped in Arc<Mutex> so it can be cloned and passed to spawn_blocking.
    pub(super) sessions: Arc<Mutex<HashMap<String, CaptureSession>>>,
    /// Engine-owned notes orchestration mode keyed by recording_id
    /// (meeting_id), parallel to `sessions`.
    ///
    /// This stays out of panops-core because capture adapters only capture;
    /// the engine decides whether `recording.stop` should enqueue notes.
    pub(super) auto_notes: Arc<Mutex<HashMap<String, bool>>>,
}

#[async_trait::async_trait]
impl IpcServer for IpcImpl {
    async fn notes_generate(
        &self,
        params: NotesGenerateParams,
    ) -> Result<JobAccepted, ErrorObjectOwned> {
        Ok(enqueue_notes_job(
            self.services.clone(),
            self.events_tx.clone(),
            params,
        ))
    }

    async fn notes_save(&self, params: NotesSaveParams) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        let data_dir = self.services.data_dir.clone();
        spawn_blocking_into_ipc("notes.save", move || {
            // Verify meeting exists AND derive the safe on-disk dir
            // from the registry row (same defense as notes.generate
            // and meeting.delete — never trust a path-like id, never
            // follow a tampered dir_path).
            let m = storage
                .get_meeting(&params.meeting_id)
                .map_err(IpcError::from)?;
            let meeting_dir = validate_meeting_dir(&data_dir, &m.dir_path)?;
            let notes_path = meeting_dir.join("notes.md");

            // Atomic write: temp sibling + rename, so a crash
            // mid-write can't leave a partial notes.md that the
            // rendered view would then fail to parse. Same pattern
            // as `write_structured_notes_json` below.
            // Unique temp suffix so a concurrent notes.save for the same
            // meeting can't clobber another's in-flight partial file.
            let partial = meeting_dir.join(format!("notes.md.{}.partial", uuid::Uuid::new_v4()));
            std::fs::write(&partial, params.markdown.as_bytes()).map_err(|e| {
                tracing::error!(
                    error = %e,
                    path = ?partial,
                    "notes.save: write notes.md.partial failed"
                );
                IpcError::Internal {
                    message: "write notes.md failed".into(),
                }
            })?;
            std::fs::rename(&partial, &notes_path).map_err(|e| {
                tracing::error!(
                    error = %e,
                    from = ?partial,
                    to = ?notes_path,
                    "notes.save: rename notes.md.partial -> notes.md failed"
                );
                IpcError::Internal {
                    message: "save notes.md failed".into(),
                }
            })?;

            // Replace the meeting's note row with a single fresh row
            // pointing at the just-written file. Dialect is "basic"
            // because user-edited markdown is unstructured.
            let meeting_id = params.meeting_id.clone();
            let draft = panops_core::storage::NoteDraft {
                id: uuid::Uuid::new_v4().simple().to_string(),
                meeting_id: params.meeting_id,
                dialect: "basic".into(),
                content_md: params.markdown,
                primary_path: notes_path.to_string_lossy().into_owned(),
            };
            storage
                .replace_meeting_note(&meeting_id, draft)
                .map_err(IpcError::from)?;

            Ok(())
        })
        .await
    }

    async fn meeting_list(
        &self,
        params: Option<MeetingListParams>,
    ) -> Result<Vec<MeetingSummary>, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("meeting.list", move || {
            let rows = match params {
                Some(params) => storage
                    .list_meetings_filtered(params.into())
                    .map_err(IpcError::from)?,
                None => storage.list_meetings().map_err(IpcError::from)?,
            };
            Ok(rows.into_iter().map(MeetingSummary::from).collect())
        })
        .await
    }

    async fn server_info(&self) -> Result<ServerInfo, ErrorObjectOwned> {
        Ok(ServerInfo {
            llm: self.services.llm_info.clone(),
        })
    }

    async fn meeting_start(&self, params: MeetingConfig) -> Result<String, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        let data_dir = self.services.data_dir.clone();
        spawn_blocking_into_ipc("meeting.start", move || {
            let (id, _dir) = create_meeting_dir_and_row(
                storage.as_ref(),
                &data_dir,
                params.title.unwrap_or_default(),
                params.language.unwrap_or_else(|| "auto".into()),
            )?;
            Ok(id)
        })
        .await
    }

    async fn meeting_stop(&self, params: MeetingIdParam) -> Result<Meeting, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        let id = params.id;
        spawn_blocking_into_ipc("meeting.stop", move || {
            // Read the existing row to compute duration_ms from
            // started_at -> now. NotFound surfaces immediately as
            // InputNotFound (no need to attempt the update).
            let m = storage.get_meeting(&id).map_err(IpcError::from)?;
            let ended_at = chrono::Local::now().fixed_offset().to_rfc3339();
            let started = chrono::DateTime::parse_from_rfc3339(&m.started_at).map_err(|e| {
                tracing::error!(error = %e, "meeting.stop parse started_at");
                IpcError::Internal {
                    message: "internal time-parse error".into(),
                }
            })?;
            let ended = chrono::DateTime::parse_from_rfc3339(&ended_at).map_err(|e| {
                // Should be unreachable — we just formatted ended_at
                // ourselves with chrono. If chrono ever changes its
                // round-trip contract, this surfaces as Internal
                // instead of panicking + poisoning shared state.
                tracing::error!(error = %e, "meeting.stop parse self-formatted ended_at");
                IpcError::Internal {
                    message: "internal time-format error".into(),
                }
            })?;
            let dur = (ended - started).num_milliseconds().max(0) as u64;
            let updated = storage
                .update_meeting_ended(&id, &ended_at, dur)
                .map_err(IpcError::from)?;
            Ok(to_protocol_meeting(updated))
        })
        .await
    }

    async fn meeting_get(&self, params: MeetingIdParam) -> Result<Meeting, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        let id = params.id;
        spawn_blocking_into_ipc("meeting.get", move || {
            storage
                .get_meeting(&id)
                .map(to_protocol_meeting)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn meeting_set_language(
        &self,
        params: MeetingSetLanguageParams,
    ) -> Result<Meeting, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("meeting.set_language", move || {
            storage
                .update_meeting_language(&params.id, &params.language)
                .map(to_protocol_meeting)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn meeting_rename(
        &self,
        params: MeetingRenameParams,
    ) -> Result<Meeting, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("meeting.rename", move || {
            storage
                .rename_meeting(&params.meeting_id, &params.title)
                .map(to_protocol_meeting)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn meeting_delete(&self, params: MeetingIdParam) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        let data_dir = self.services.data_dir.clone();
        let id = params.id;
        spawn_blocking_into_ipc("meeting.delete", move || {
            // Read first to capture dir_path. Deleting the row before
            // the fs cleanup keeps the registry the source of truth
            // (orphan dir on fs failure is recoverable; orphan rows
            // are surprises waiting to bite later).
            let m = storage.get_meeting(&id).map_err(IpcError::from)?;
            // Defense against a tampered `panops.db` whose `dir_path`
            // points outside `<data_dir>/meetings/`. Without this,
            // `remove_dir_all(/Users/fran/Code)` would be one row edit
            // away. We refuse to delete the row at all if the path
            // doesn't validate — better an orphan row than catastrophic
            // recursive delete.
            let safe_dir = validate_meeting_dir(&data_dir, &m.dir_path)?;
            storage.delete_meeting(&id).map_err(IpcError::from)?;
            if let Err(e) = std::fs::remove_dir_all(&safe_dir) {
                tracing::warn!(
                    error = %e,
                    dir = ?safe_dir,
                    "meeting.delete: row gone but fs cleanup failed (orphan dir)"
                );
            }
            Ok(())
        })
        .await
    }

    async fn meeting_assign(&self, params: MeetingAssignParams) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("meeting.assign", move || {
            storage
                .assign_meeting(&params.meeting_id, params.space_id, params.project_id)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn meeting_delete_video(
        &self,
        params: MeetingDeleteVideoParams,
    ) -> Result<MeetingDeleteVideoResult, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        let data_dir = self.services.data_dir.clone();
        let meeting_id = params.meeting_id;
        spawn_blocking_into_ipc("meeting.deleteVideo", move || {
            reject_path_like_meeting_id(&meeting_id)?;
            let m = storage.get_meeting(&meeting_id).map_err(IpcError::from)?;
            let meeting_dir = validate_meeting_dir(&data_dir, &m.dir_path)?;
            let video_path = meeting_dir.join("recording.mov");

            let freed_bytes = match std::fs::metadata(&video_path) {
                Ok(meta) => meta.len(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(MeetingDeleteVideoResult {
                        deleted: false,
                        freed_bytes: 0,
                    });
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        path = ?video_path,
                        "meeting.deleteVideo: stat recording.mov failed"
                    );
                    return Err(IpcError::Internal {
                        message: "delete video failed".into(),
                    });
                }
            };

            match std::fs::remove_file(&video_path) {
                Ok(()) => Ok(MeetingDeleteVideoResult {
                    deleted: true,
                    freed_bytes,
                }),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok(MeetingDeleteVideoResult {
                        deleted: false,
                        freed_bytes: 0,
                    })
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        path = ?video_path,
                        "meeting.deleteVideo: remove recording.mov failed"
                    );
                    Err(IpcError::Internal {
                        message: "delete video failed".into(),
                    })
                }
            }
        })
        .await
    }

    async fn space_create(&self, params: SpaceCreateParams) -> Result<Space, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("space.create", move || {
            storage
                .create_space(&params.name)
                .map(Space::from)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn space_list(&self) -> Result<SpaceListResult, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("space.list", move || {
            let spaces = storage
                .list_spaces()
                .map_err(IpcError::from)?
                .into_iter()
                .map(Space::from)
                .collect();
            Ok(SpaceListResult { spaces })
        })
        .await
    }

    async fn space_rename(&self, params: SpaceRenameParams) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("space.rename", move || {
            storage
                .rename_space(&params.id, &params.name)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn space_delete(&self, params: SpaceDeleteParams) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("space.delete", move || {
            storage.delete_space(&params.id).map_err(IpcError::from)
        })
        .await
    }

    async fn project_create(
        &self,
        params: ProjectCreateParams,
    ) -> Result<Project, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("project.create", move || {
            storage
                .create_project(&params.space_id, &params.name)
                .map(Project::from)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn project_list(
        &self,
        params: Option<ProjectListParams>,
    ) -> Result<ProjectListResult, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("project.list", move || {
            let space_id = params.as_ref().and_then(|p| p.space_id.as_deref());
            let projects = storage
                .list_projects(space_id)
                .map_err(IpcError::from)?
                .into_iter()
                .map(Project::from)
                .collect();
            Ok(ProjectListResult { projects })
        })
        .await
    }

    async fn project_rename(&self, params: ProjectRenameParams) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("project.rename", move || {
            storage
                .rename_project(&params.id, &params.name)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn project_delete(&self, params: ProjectDeleteParams) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("project.delete", move || {
            storage.delete_project(&params.id).map_err(IpcError::from)
        })
        .await
    }

    async fn tag_create(&self, params: TagCreateParams) -> Result<Tag, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("tag.create", move || {
            storage
                .create_tag(&params.name)
                .map(Tag::from)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn tag_list(&self) -> Result<TagListResult, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("tag.list", move || {
            let tags = storage
                .list_tags()
                .map_err(IpcError::from)?
                .into_iter()
                .map(Tag::from)
                .collect();
            Ok(TagListResult { tags })
        })
        .await
    }

    async fn tag_delete(&self, params: TagDeleteParams) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("tag.delete", move || {
            storage.delete_tag(&params.id).map_err(IpcError::from)
        })
        .await
    }

    async fn tag_assign(&self, params: TagAssignParams) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("tag.assign", move || {
            storage
                .tag_meeting(&params.meeting_id, &params.tag_id)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn tag_unassign(&self, params: TagAssignParams) -> Result<(), ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        spawn_blocking_into_ipc("tag.unassign", move || {
            storage
                .untag_meeting(&params.meeting_id, &params.tag_id)
                .map_err(IpcError::from)
        })
        .await
    }

    async fn recording_start(
        &self,
        params: RecordingStartParams,
    ) -> Result<RecordingAccepted, ErrorObjectOwned> {
        let capture = crate::capture_resolver::pick_capture();
        let storage = self.services.storage.clone();
        let data_dir = self.services.data_dir.clone();
        let meeting_id = params.meeting_id.clone();
        let auto_generate_notes = params.auto_generate_notes;

        // Clone engine-side stores for use in spawn_blocking.
        let sessions = self.sessions.clone();
        let auto_notes = self.auto_notes.clone();

        spawn_blocking_into_ipc("recording.start", move || {
            // Verify meeting exists.
            let m = storage.get_meeting(&meeting_id).map_err(IpcError::from)?;
            let meeting_dir = validate_meeting_dir(&data_dir, &m.dir_path)?;

            // Start capture with config from wire params.
            let config = panops_core::capture::CaptureConfig::from(&params);
            let session = capture
                .start_capture(&meeting_id, &meeting_dir, &config)
                .map_err(IpcError::from)?;

            // Persist session for recording.stop to look up.
            sessions
                .lock()
                .map_err(|_| IpcError::Internal {
                    message: "sessions mutex poisoned".into(),
                })?
                .insert(meeting_id.clone(), session);
            auto_notes
                .lock()
                .map_err(|_| IpcError::Internal {
                    message: "auto_notes mutex poisoned".into(),
                })?
                .insert(meeting_id.clone(), auto_generate_notes);

            // Return recording_id as meeting_id.
            Ok(RecordingAccepted {
                recording_id: meeting_id,
            })
        })
        .await
    }

    async fn recording_stop(
        &self,
        params: RecordingStopParams,
    ) -> Result<RecordingStopped, ErrorObjectOwned> {
        let capture = crate::capture_resolver::pick_capture();
        let recording_id = params.recording_id;

        // Clone engine-side stores for use in spawn_blocking.
        let sessions = self.sessions.clone();
        let auto_notes = self.auto_notes.clone();
        let services = self.services.clone();
        let events_tx = self.events_tx.clone();

        let (mut stopped, auto_generate_notes, recording_id) =
            spawn_blocking_into_ipc("recording.stop", move || {
                let auto_generate_notes = auto_notes
                    .lock()
                    .map_err(|_| IpcError::Internal {
                        message: "auto_notes mutex poisoned".into(),
                    })?
                    .remove(&recording_id)
                    .unwrap_or(false);

                // Look up the real session persisted by recording.start.
                let session = sessions
                    .lock()
                    .map_err(|_| IpcError::Internal {
                        message: "sessions mutex poisoned".into(),
                    })?
                    .remove(&recording_id)
                    .ok_or_else(|| IpcError::InputNotFound {
                        path: format!("session/{recording_id}"),
                    })?;

                let result = capture.stop_capture(&session).map_err(IpcError::from)?;

                let stopped = RecordingStopped {
                    system_audio_path: result
                        .system_audio_path
                        .as_ref()
                        .map(|p| p.display().to_string()),
                    mic_audio_path: result
                        .mic_audio_path
                        .as_ref()
                        .map(|p| p.display().to_string()),
                    screenshot_paths: result
                        .screenshot_paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect(),
                    duration_ms: result.duration_ms,
                    // Set below once auto-generate (if requested) has enqueued
                    // a job — capture-only stop never knows a notes job id.
                    notes_job_id: None,
                };

                Ok((stopped, auto_generate_notes, recording_id))
            })
            .await?;

        // When auto-generate is set, surface the enqueued notes job id on the
        // stop result so the app can track it through the same job.done /
        // job.error flow as a manual `notes.generate`. Stays `None` when
        // compute wasn't ready (warmup / no provider) — the app then shows a
        // "notes deferred" hint and leaves the meeting manually generable.
        if auto_generate_notes {
            stopped.notes_job_id =
                maybe_enqueue_auto_notes(services, events_tx, &recording_id, &stopped);
        }

        Ok(stopped)
    }

    async fn capture_windows(
        &self,
        _params: CaptureWindowsParams,
    ) -> Result<CaptureWindowsResult, ErrorObjectOwned> {
        let capture = crate::capture_resolver::pick_capture();
        spawn_blocking_into_ipc("capture.windows", move || {
            let windows = capture.list_windows().map_err(IpcError::from)?;
            Ok(CaptureWindowsResult {
                windows: windows.into_iter().map(WindowInfo::from).collect(),
            })
        })
        .await
    }

    async fn subscribe_events(&self, pending: PendingSubscriptionSink) -> SubscriptionResult {
        let sink = pending.accept().await?;
        let mut rx = self.events_tx.subscribe();
        loop {
            tokio::select! {
                _ = sink.closed() => break,
                event = rx.recv() => {
                    match event {
                        Ok(e) => {
                            let raw = match serde_json::value::to_raw_value(&e) {
                                Ok(r) => r,
                                Err(err) => {
                                    tracing::warn!(error = ?err, "drop event with bad serialise");
                                    continue;
                                }
                            };
                            if sink.send(raw).await.is_err() {
                                break;
                            }
                        }
                        // Lagged: a slow consumer fell behind the broadcast
                        // ring. We skip and keep the subscription open
                        // because losing one event is better than tearing
                        // down the connection.
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(skipped, "events subscriber lagged");
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
        Ok(())
    }
}

/// Accept a notes job and run it through the single notes.generate event path.
///
/// Both explicit `ipc.notes.generate` and automatic post-recording processing
/// use this helper so backpressure, panic-to-`job.error` conversion, progress
/// events, and `job.done`/`job.error` payloads cannot drift.
fn enqueue_notes_job(
    services: Arc<crate::server::EngineServices>,
    events_tx: broadcast::Sender<Event>,
    params: NotesGenerateParams,
) -> JobAccepted {
    let job_id = uuid::Uuid::new_v4().to_string();
    let job_id_owned = job_id.clone();

    // Accept the job and return its id immediately. ALL heavy work — the
    // backpressure wait AND the synchronous pipeline — runs off the RPC
    // accept path, so the `{job_id}`-then-events contract holds even under
    // load: the caller never blocks waiting for a permit. Results land on
    // `events.subscribe` as `JobDone` or `JobError`.
    tokio::spawn(async move {
        // Backpressure heavy note generation before it reaches tokio's
        // blocking pool. A permit is held for the whole synchronous
        // pipeline, so a burst of jobs cannot enqueue unbounded decoded
        // audio / ASR / diarization / LLM work and exhaust RAM. Jobs
        // beyond `MAX_CONCURRENT_NOTES_JOBS` wait here (off the RPC path).
        let notes_job_permit = match services.notes_jobs.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(e) => {
                // `acquire_owned` errors only when the semaphore is
                // permanently closed (engine shutdown) — not the normal
                // wait-when-full case. The job id was already returned, so
                // surface the failure as a `JobError` event, not an RPC error.
                tracing::error!(error = %e, "notes.generate semaphore permanently closed");
                let _ = events_tx.send(Event::JobError(JobErrorEvent {
                    job_id: job_id_owned,
                    error: IpcError::Internal {
                        message: "notes.generate internal error".into(),
                    },
                }));
                return;
            }
        };

        // Move the pipeline off any tokio worker thread: rayon (used by
        // `NotesGenerator` for the per-section fan-out) and the blocking
        // ASR/diar adapters mustn't share a runtime worker with the RPC
        // accept loop. `spawn_blocking` drops them on the dedicated
        // blocking pool.
        let job_id_for_panic = job_id_owned.clone();
        let events_tx_for_panic = events_tx.clone();
        let join_handle = tokio::task::spawn_blocking(move || {
            let _notes_job_permit = notes_job_permit;
            let outcome = run_notes_pipeline(&services, &params, &events_tx, &job_id_owned);
            match outcome {
                Ok(result) => {
                    let _ = events_tx.send(Event::JobDone(JobDoneEvent {
                        job_id: job_id_owned,
                        result,
                    }));
                }
                Err(error) => {
                    let _ = events_tx.send(Event::JobError(JobErrorEvent {
                        job_id: job_id_owned,
                        error,
                    }));
                }
            }
        });

        // Awaiter for the blocking task. Without this, a panic inside
        // the closure (MockLlm fingerprint mismatch, rayon panic, OOM)
        // is silently swallowed when the JoinHandle drops, leaving
        // subscribers waiting forever. We turn a JoinError into a
        // synthetic `JobError` event with an opaque `Internal` message
        // so the wire never leaks panic payloads or filesystem paths.
        if let Err(join_err) = join_handle.await {
            let msg = if join_err.is_panic() {
                "pipeline panicked".to_string()
            } else {
                format!("pipeline cancelled: {join_err}")
            };
            tracing::error!(error = %join_err, "notes.generate pipeline did not complete");
            let _ = events_tx_for_panic.send(Event::JobError(JobErrorEvent {
                job_id: job_id_for_panic,
                error: IpcError::Internal { message: msg },
            }));
        }
    });

    JobAccepted { job_id }
}

/// Enqueue the post-recording notes job when auto-generate is on and compute is
/// ready, returning the job id so `recording.stop` can echo it to the client.
/// Returns `None` (and leaves the meeting manually generable) when compute isn't
/// ready or no captured audio is available — the caller then leaves
/// `notes_job_id` unset so the client can surface a deferred hint.
fn maybe_enqueue_auto_notes(
    services: Arc<crate::server::EngineServices>,
    events_tx: broadcast::Sender<Event>,
    meeting_id: &str,
    stopped: &RecordingStopped,
) -> Option<String> {
    if let Err(reason) = auto_notes_compute_available(&services) {
        tracing::warn!(
            meeting_id,
            reason = %reason,
            "auto-generate-notes skipped: no LLM provider available; meeting left for manual generation"
        );
        return None;
    }

    let Some(audio) = stopped
        .system_audio_path
        .as_ref()
        .or(stopped.mic_audio_path.as_ref())
        .cloned()
    else {
        tracing::warn!(
            meeting_id,
            "auto-generate-notes skipped: no captured audio path; meeting left for manual generation"
        );
        return None;
    };

    let language = match services.storage.get_meeting(meeting_id) {
        Ok(meeting) if meeting.language.trim().is_empty() || meeting.language == "auto" => None,
        Ok(meeting) => Some(meeting.language),
        Err(e) => {
            tracing::warn!(
                meeting_id,
                error = %e,
                "auto-generate-notes could not read meeting language; using automatic language detection"
            );
            None
        }
    };

    let accepted = enqueue_notes_job(
        services,
        events_tx,
        NotesGenerateParams {
            audio,
            dialect: None,
            llm_provider: None,
            llm_model: None,
            no_diarize: None,
            language,
            meeting_id: Some(meeting_id.to_string()),
        },
    );
    tracing::info!(
        meeting_id,
        job_id = %accepted.job_id,
        "auto-generate-notes enqueued notes.generate job"
    );
    Some(accepted.job_id)
}

fn auto_notes_compute_available(services: &crate::server::EngineServices) -> Result<(), String> {
    if services.llm_info.provider.trim().is_empty() {
        return Err("resolved LLM provider is empty".into());
    }
    match services.heavy.get() {
        Some(Ok(_)) => Ok(()),
        Some(Err(msg)) => Err(format!("heavy adapter init failed: {msg}")),
        None => Err("engine AI adapters are still warming up".into()),
    }
}

fn emit_job_progress(
    events_tx: &broadcast::Sender<Event>,
    job_id: &str,
    stage: &str,
    current: Option<u32>,
    total: Option<u32>,
    message: Option<&str>,
) {
    let _ = events_tx.send(Event::JobProgress(JobProgressEvent {
        job_id: job_id.to_string(),
        stage: stage.to_string(),
        current,
        total,
        message: message.map(str::to_string),
    }));
}

/// Run VAD + recursive ASR over one already-loaded track, returning its
/// stitched segments (absolute timestamps) and the model name. Mirrors the
/// legacy single-track loop so both the file-import path and the slice-11
/// two-track path share one transcription routine.
fn transcribe_track(
    heavy: &crate::server::HeavyAdapters,
    samples: &[f32],
    sample_rate: u32,
    language: Option<&str>,
    events_tx: &broadcast::Sender<Event>,
    job_id: &str,
) -> Result<(Vec<panops_core::Segment>, Option<String>), IpcError> {
    transcribe_track_labeled(
        heavy,
        samples,
        sample_rate,
        language,
        events_tx,
        job_id,
        None,
    )
}

fn transcribe_track_labeled(
    heavy: &crate::server::HeavyAdapters,
    samples: &[f32],
    sample_rate: u32,
    language: Option<&str>,
    events_tx: &broadcast::Sender<Event>,
    job_id: &str,
    message: Option<&str>,
) -> Result<(Vec<panops_core::Segment>, Option<String>), IpcError> {
    let regions = heavy.vad.detect_speech(samples, sample_rate).map_err(|e| {
        tracing::error!(error = %e, "vad detect_speech failed");
        IpcError::from(e)
    })?;
    let merged = panops_portable::audio::merge_adjacent_regions(regions, 5_000);
    let total_audio_ms = (samples.len() as u64 * 1000) / u64::from(sample_rate);
    let mut segments = Vec::new();
    let mut model = None;
    let total = merged.len() as u32;
    for (idx, region) in merged.iter().enumerate() {
        let clamped = region.clamp_to(total_audio_ms);
        if clamped.start_ms >= clamped.end_ms {
            continue;
        }
        emit_job_progress(
            events_tx,
            job_id,
            "transcribing",
            Some((idx + 1) as u32),
            Some(total),
            message,
        );
        let result = panops_portable::recursive_asr::transcribe_recursive(
            heavy.asr.as_ref(),
            samples,
            sample_rate,
            clamped,
            language,
        )
        .map_err(|e| {
            tracing::error!(error = %e, "transcribe_recursive failed");
            IpcError::from(e)
        })?;
        if model.is_none() {
            model = result.model;
        }
        segments.extend(result.segments);
    }
    Ok((segments, model))
}

/// Two-track capture pipeline (slice 11). Transcribes whichever of the two
/// capture tracks are present, pins the mic track to the local speaker
/// (id 0, "You"), diarizes only the system track for remote speakers
/// (ids >= 1, offset past the local id), and merges both by timestamp via
/// [`panops_core::merge_two_track`]. At least one track must be present.
fn transcribe_two_track(
    heavy: &crate::server::HeavyAdapters,
    system_wav: Option<&std::path::Path>,
    mic_wav: Option<&std::path::Path>,
    language: Option<&str>,
    no_diarize: bool,
    events_tx: &broadcast::Sender<Event>,
    job_id: &str,
) -> Result<panops_core::Transcript, IpcError> {
    if system_wav.is_none() && mic_wav.is_none() {
        return Err(IpcError::InvalidInput {
            message: "two-track transcription requires at least one track".into(),
        });
    }

    let mut mic_segments = Vec::new();
    let mut system_segments = Vec::new();
    let mut system_turns = Vec::new();
    let mut model = None;
    let mut duration_ms = 0u64;

    if let Some(mic) = mic_wav {
        let (samples, sr) =
            panops_portable::audio::load_audio_mono16k(mic).map_err(IpcError::from)?;
        duration_ms = duration_ms.max((samples.len() as u64 * 1000) / u64::from(sr));
        let (segs, m) = transcribe_track_labeled(
            heavy,
            &samples,
            sr,
            language,
            events_tx,
            job_id,
            Some("mic track"),
        )?;
        if model.is_none() {
            model = m;
        }
        mic_segments = segs;
    }
    if let Some(system) = system_wav {
        let (samples, sr) =
            panops_portable::audio::load_audio_mono16k(system).map_err(IpcError::from)?;
        duration_ms = duration_ms.max((samples.len() as u64 * 1000) / u64::from(sr));
        let (segs, m) = transcribe_track_labeled(
            heavy,
            &samples,
            sr,
            language,
            events_tx,
            job_id,
            Some("system track"),
        )?;
        if model.is_none() {
            model = m;
        }
        system_segments = segs;
        if !no_diarize {
            emit_job_progress(events_tx, job_id, "diarizing", None, None, None);
            system_turns = heavy.diar.diarize(system).map_err(IpcError::from)?;
        }
    }

    let segments = panops_core::merge_two_track(mic_segments, system_segments, &system_turns);
    Ok(panops_core::Transcript {
        schema_version: panops_core::Transcript::SCHEMA_VERSION,
        model: model.unwrap_or_else(|| "vad-multilingual".to_string()),
        audio_path: system_wav
            .or(mic_wav)
            .map(|p| p.to_path_buf())
            .unwrap_or_default(),
        audio_duration_ms: duration_ms,
        diarized: !no_diarize,
        segments,
    })
}

/// Synchronous core of `notes.generate`. Runs on the blocking pool and
/// mirrors `panops-engine`'s CLI `run_notes` flow: ASR -> optional
/// diarization merge -> `NotesGenerator` -> `MarkdownExporter`. All
/// domain errors (`AsrError`, `DiarError`, `LlmError`, `NotesError`,
/// `ExportError`) map to `IpcError` via the `domain-conversions`
/// feature on `panops-protocol`.
///
/// Readiness gate (eager-after-bind, closes #74): heavy adapters live
/// in `services.heavy` (see `EngineServices::pending`). Until the
/// background init task fills that lock, this function returns
/// `IpcError::ProviderUnavailable` so clients get an explicit "warming
/// up" signal instead of a 20-second silent stall. Tests build via
/// `EngineServices::ready` which pre-fills the lock, so the gate is a
/// no-op for them.
pub(super) fn run_notes_pipeline(
    services: &crate::server::EngineServices,
    params: &NotesGenerateParams,
    events_tx: &broadcast::Sender<Event>,
    job_id: &str,
) -> Result<NotesGenerateResult, IpcError> {
    // Warmup gate first. The CLI `serve` path uses
    // `EngineServices::pending(llm)` and fills `heavy` from a
    // `spawn_blocking` task that runs concurrent with the accept
    // loop — until it sets the lock, return `ProviderUnavailable` so
    // the client can retry. Test paths use `EngineServices::ready`
    // which pre-fills with `Ok(...)`, skipping the wait entirely.
    let heavy = match services.heavy.get() {
        Some(Ok(h)) => h,
        Some(Err(msg)) => {
            tracing::error!(error = %msg, "heavy adapter init reported failure");
            return Err(IpcError::Internal {
                message: format!("adapter init failed: {msg}"),
            });
        }
        None => {
            return Err(IpcError::ProviderUnavailable {
                message: "engine warming up; retry shortly".into(),
            });
        }
    };

    // Reject empty audio strings outright — `PathBuf::from("")` is
    // technically valid but canonicalize-on-empty depends on platform
    // and gives unhelpful errors. Empty/blank input is a validation
    // failure, not a missing-file failure, so map it to `InvalidInput`
    // (the absent path field on `InputNotFound` would be useless here).
    if params.audio.trim().is_empty() {
        return Err(IpcError::InvalidInput {
            message: "audio path is empty".into(),
        });
    }

    // Canonicalize BEFORE any pipeline work. This both:
    //   1. Closes the `audio="../../etc/passwd"` traversal vector — the
    //      computed `out_dir = parent.join("<stem>-notes")` is now
    //      anchored to the canonical (absolute, symlink-resolved)
    //      directory of the audio file, so `..` in the input cannot
    //      walk above the real parent.
    //   2. Surfaces missing-input synchronously, before the ASR adapter
    //      observes the path. The wire-level error stays
    //      `InputNotFound` (the same kind the ASR-not-found path emits)
    //      and reflects the user-supplied string, not the canonical FS
    //      layout.
    // We deliberately don't add an allowlist (e.g. "must live under
    // ~/Library/Application Support/panops") because the slice-04
    // fixtures live under `tests/fixtures/audio/` and the slice-05
    // threat model only requires closing traversal.
    let raw_audio_path = PathBuf::from(&params.audio);
    let audio_path = std::fs::canonicalize(&raw_audio_path).map_err(|e| {
        tracing::error!(
            error = %e,
            path = ?raw_audio_path,
            "notes.generate canonicalize failed"
        );
        IpcError::InputNotFound {
            path: params.audio.clone(),
        }
    })?;
    emit_job_progress(events_tx, job_id, "loading", None, None, None);

    // Slice 06: resolve `meeting_id`. If the caller passed one, verify
    // it exists (NotFound -> InputNotFound surfaces synchronously).
    // Otherwise, auto-create the meeting now so the exporter writes
    // into the canonical `meetings/<uuid>/` layout. The created
    // meeting's `started_at` is server-set; `title` defaults to the
    // audio file stem.
    let (resolved_meeting_id, canonical_out_dir) = match &params.meeting_id {
        Some(id) => {
            let m = services.storage.get_meeting(id).map_err(IpcError::from)?;
            // Same defense as `meeting.delete`: a tampered registry
            // could route note output to an arbitrary location.
            let safe_dir = validate_meeting_dir(&services.data_dir, &m.dir_path)?;
            (id.clone(), safe_dir)
        }
        None => {
            let title = audio_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("untitled")
                .to_string();
            create_meeting_dir_and_row(
                services.storage.as_ref(),
                &services.data_dir,
                title,
                params.language.clone().unwrap_or_else(|| "auto".into()),
            )?
        }
    };

    // Slice 07/11: VAD-aware multilingual transcription.
    //
    // Slice 11 adds the two-track live-capture path. A capture meeting
    // writes two tracks into its meeting dir — `system.wav` (remote
    // participants) and `mic.wav` (the local user). When either is
    // present we take `transcribe_two_track`: the mic track is pinned to
    // the local speaker (id 0, "You") and only the system track is
    // diarized for remote speakers (ids >= 1), then both are merged by
    // timestamp. File-import meetings (no capture WAVs) keep the legacy
    // single-track path unchanged:
    //   1. Load samples once (16 kHz mono).
    //   2. Run VAD to find speech regions.
    //   3. Merge adjacent regions (gap < 5s) so each region is long
    //      enough for Whisper's per-call language detection.
    //   4. Transcribe each merged region with per-region auto-detect.
    //   5. Stitch transcripts back with absolute-time offsets, then
    //      optionally diarize the whole file with sherpa.
    let system_wav = canonical_out_dir.join("system.wav");
    let mic_wav = canonical_out_dir.join("mic.wav");
    let two_track = system_wav.exists() || mic_wav.exists();

    let transcript = if two_track {
        if !audio_path.starts_with(&canonical_out_dir) {
            tracing::warn!(
                audio_path = ?audio_path,
                meeting_dir = ?canonical_out_dir,
                "explicit audio file is being bypassed in favor of capture WAVs in the meeting dir"
            );
        }
        transcribe_two_track(
            heavy,
            system_wav.exists().then_some(system_wav.as_path()),
            mic_wav.exists().then_some(mic_wav.as_path()),
            params.language.as_deref(),
            params.no_diarize.unwrap_or(false),
            events_tx,
            job_id,
        )?
    } else {
        let (samples, sample_rate) =
            panops_portable::audio::load_audio_mono16k(&audio_path).map_err(IpcError::from)?;
        let total_audio_ms = (samples.len() as u64 * 1000) / u64::from(sample_rate);
        let (segments, model) = transcribe_track(
            heavy,
            &samples,
            sample_rate,
            params.language.as_deref(),
            events_tx,
            job_id,
        )?;

        let mut transcript = panops_core::Transcript {
            schema_version: panops_core::Transcript::SCHEMA_VERSION,
            model: model.unwrap_or_else(|| "vad-multilingual".to_string()),
            audio_path: audio_path.clone(),
            audio_duration_ms: total_audio_ms,
            diarized: false,
            segments,
        };

        let no_diarize = params.no_diarize.unwrap_or(false);
        if !no_diarize {
            emit_job_progress(events_tx, job_id, "diarizing", None, None, None);
            let turns = heavy.diar.diarize(&audio_path).map_err(IpcError::from)?;
            transcript.segments = merge_speaker_turns(transcript.segments, &turns);
            transcript.diarized = true;
        }
        transcript
    };
    // Slice 06: write into the canonical `meetings/<uuid>/` layout
    // (resolved above). The on-disk `screenshots/` subdir was already
    // created during meeting.start (or during auto-create above) so
    // we only need to make sure the dir itself exists for the
    // existing-meeting path.
    let out_dir = canonical_out_dir;
    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            tracing::error!(
                error = %e,
                path = ?out_dir,
                "notes.generate failed to create output directory"
            );
            IpcError::Internal {
                message: "failed to prepare output directory".into(),
            }
        })?;
    }

    // Drop the transcript JSON before LLM generation so a downstream
    // LLM failure (missing API key, ollama down, prompt too long)
    // doesn't wipe the only proof that ASR + diarization completed.
    // Best-effort: a write failure logs but doesn't block the pipeline.
    match serde_json::to_string_pretty(&transcript) {
        Ok(json) => {
            let transcript_path = out_dir.join("transcript.json");
            if let Err(e) = std::fs::write(&transcript_path, json) {
                tracing::warn!(
                    error = %e,
                    path = ?transcript_path,
                    "notes.generate: write transcript.json failed; continuing"
                );
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "notes.generate: serialize transcript.json failed");
        }
    }

    // Additive raw-transcript sidecar (transcript.txt): a human-readable,
    // grep-friendly view of the raw Whisper segments alongside notes.md, so the
    // LLM synthesis can be compared against the source. Best-effort, mirroring
    // the transcript.json write above — a failure logs but never aborts.
    let transcript_txt_path = match write_raw_transcript(&transcript.segments, &out_dir) {
        Ok(p) => {
            tracing::info!(file = ?p, "notes.generate: wrote raw transcript sidecar");
            Some(p.display().to_string())
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "notes.generate: write transcript.txt failed; continuing"
            );
            None
        }
    };

    let dialect = match params.dialect {
        Some(NotesDialect::Basic) => MarkdownDialect::Basic,
        Some(NotesDialect::NotionEnhanced) | None => MarkdownDialect::NotionEnhanced,
    };

    let started_at = chrono::Local::now().fixed_offset();
    let input = NotesInput {
        transcript: transcript.segments,
        screenshots: Vec::new(),
        meeting_metadata: MeetingMetadata {
            started_at,
            duration_ms: transcript.audio_duration_ms,
            // Use the actually-transcribed source (the capture WAV on the
            // two-track path, the file on the single-track path) so the notes
            // frontmatter never points at a bypassed `params.audio` file.
            source_path: Some(transcript.audio_path.clone()),
            language_hint: params.language.clone(),
        },
    };

    emit_job_progress(events_tx, job_id, "generating_notes", None, None, None);
    let generator = NotesGenerator {
        llm: services.llm.as_ref(),
        dialect,
    };
    let notes = generator.generate(input).map_err(IpcError::from)?;
    let exporter = heavy.exporter.clone();

    emit_job_progress(events_tx, job_id, "exporting", None, None, None);
    let artifact = exporter.export(&notes, &out_dir).map_err(|e| {
        // Domain-to-wire mapping lives in `panops-protocol` (gated by
        // `domain-conversions`); we still log the full error here so
        // the operator sees template / FS detail that the wire-side
        // message intentionally hides.
        tracing::error!(error = %e, "notes.generate exporter failed");
        IpcError::from(e)
    })?;

    if let Err(e) = write_structured_notes_json(&notes, &out_dir) {
        tracing::warn!(error = %e, "notes.generate: write notes.json failed; continuing");
    }

    // Persist the note row. Genuinely best-effort: the markdown file
    // is already on disk at `artifact.primary_file`. If the registry
    // insert fails (e.g. FK violation because the meeting was
    // deleted mid-flight, mutex poisoned, etc.), the client should
    // still get JobDone with the path to the file we just wrote —
    // hiding the artifact would be worse UX than an unregistered
    // note. The storage error goes to `tracing::error!`.
    let dialect_str = dialect.as_str();
    let content_md = match std::fs::read_to_string(&artifact.primary_file) {
        Ok(s) => s,
        Err(e) => {
            // The file existed long enough for the exporter to write
            // it; failing to read it back is unusual. Log the
            // specific failure so the operator can investigate (FS
            // permission flap, antivirus quarantine, disk error).
            // Continue with empty `content_md` rather than blocking
            // the client from learning about the artifact.
            tracing::warn!(
                error = %e,
                path = ?artifact.primary_file,
                "notes.generate could not read back artifact for registry; storing empty content"
            );
            String::new()
        }
    };
    if let Err(e) = services
        .storage
        .create_note(panops_core::storage::NoteDraft {
            id: uuid::Uuid::new_v4().simple().to_string(),
            meeting_id: resolved_meeting_id.clone(),
            dialect: dialect_str.into(),
            content_md,
            primary_path: artifact.primary_file.display().to_string(),
        })
    {
        tracing::error!(
            error = %e,
            meeting_id = %resolved_meeting_id,
            "notes.generate: registry insert failed; markdown file is on disk anyway"
        );
    }

    Ok(NotesGenerateResult {
        primary_file: artifact.primary_file.display().to_string(),
        assets: artifact
            .assets
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        meeting_id: resolved_meeting_id,
        transcript_txt_path,
    })
}

/// Additive structured-notes sidecar (`notes.json`): exposes the
/// `StructuredNotes` IR to richer clients while `notes.md` remains the
/// primary artifact.
fn write_structured_notes_json(notes: &StructuredNotes, out_dir: &Path) -> Result<PathBuf, String> {
    let json =
        serde_json::to_string_pretty(notes).map_err(|e| format!("serialize notes.json: {e}"))?;
    let notes_json_path = out_dir.join("notes.json");
    // Write to a temp sibling then rename, so a crash mid-write can't leave a
    // partial notes.json a consumer fails to parse (notes.md is the primary
    // artifact; same .partial+rename pattern as model downloads in model.rs).
    let partial = out_dir.join("notes.json.partial");
    std::fs::write(&partial, json).map_err(|e| format!("write {partial:?}: {e}"))?;
    std::fs::rename(&partial, &notes_json_path)
        .map_err(|e| format!("rename {partial:?} -> {notes_json_path:?}: {e}"))?;
    Ok(notes_json_path)
}

/// Map `IpcError` to a JSON-RPC server error (-32000) carrying the
/// typed kind in `data` and the human-readable message at top level.
/// Mirrors the spec's "Error mapping at the RPC boundary" section.
///
/// Currently unused at the wire level — `notes.generate` reports
/// errors via `JobError` events, and `meeting.list` is stubbed to
/// `Ok(vec![])`. Kept because synchronous methods added in slice 06+
/// (e.g. `meeting.get`) will need it. Removing now means re-deriving
/// the (-32000, kind, data) shape later from the spec.
pub(super) fn ipc_error_to_obj(e: IpcError) -> ErrorObjectOwned {
    let data = serde_json::to_value(&e).expect("IpcError serialise");
    ErrorObjectOwned::owned(-32000, e.to_string(), Some(data))
}

/// Allocate a fresh meeting id, create its `meetings/<id>/screenshots/`
/// directory, and insert the corresponding registry row. On row
/// insert failure, removes the freshly-created directory so we don't
/// leave an orphan that no row points at. Used by both `meeting.start`
/// and the `notes.generate` auto-create branch — extracted to keep
/// the cleanup-on-failure pattern in one place.
fn create_meeting_dir_and_row(
    storage: &dyn panops_core::storage::Storage,
    data_dir: &std::path::Path,
    title: String,
    language: String,
) -> Result<(String, PathBuf), IpcError> {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let dir = data_dir.join("meetings").join(&id);
    std::fs::create_dir_all(dir.join("screenshots")).map_err(|e| {
        tracing::error!(error = %e, "create meetings dir");
        IpcError::Internal {
            message: "create meeting dir failed".into(),
        }
    })?;
    let started_at = chrono::Local::now().fixed_offset().to_rfc3339();
    let create_result = storage.create_meeting(panops_core::storage::MeetingDraft {
        id: id.clone(),
        title,
        started_at,
        language,
        dir_path: dir.to_string_lossy().into_owned(),
    });
    if let Err(e) = create_result {
        // Row didn't commit — fs-then-row + cleanup-on-row-error
        // ordering means we have to remove the dir we just created.
        // Cleanup failure is logged but does NOT mask the original
        // storage error (registry stays the source of truth).
        if let Err(rm_err) = std::fs::remove_dir_all(&dir) {
            tracing::warn!(
                error = %rm_err,
                dir = ?dir,
                "failed to clean up meeting dir after row insert error"
            );
        }
        return Err(IpcError::from(e));
    }
    Ok((id, dir))
}

/// Reject meeting ids that could be interpreted as filesystem paths before
/// any method derives a meeting-local artifact path from the id. Generated
/// meeting ids are UUID-simple strings, so accepting only one normal path
/// component keeps the video-artifact endpoint path-traversal-proof without
/// constraining the storage layer globally.
fn reject_path_like_meeting_id(id: &str) -> Result<(), IpcError> {
    let mut components = Path::new(id).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None)
            if !id.contains('/') && !id.contains('\\') =>
        {
            Ok(())
        }
        _ => {
            tracing::warn!(meeting_id = %id, "rejected path-like meeting id");
            Err(IpcError::InvalidInput {
                message: "meeting_id must be a single path-safe component".into(),
            })
        }
    }
}

/// Validate that a `dir_path` read from the registry actually lives
/// under `<data_dir>/meetings/` before we delete it or write notes
/// into it. Defends against a tampered `panops.db` whose `dir_path`
/// column has been edited to point at e.g. `/Users/fran/Code` —
/// `remove_dir_all` on that would be catastrophic. Two checks:
///
/// 1. **Lexical** prefix match (no FS access). Catches obviously-bad
///    paths like `/etc` or `../etc/passwd` without requiring the file
///    to exist (so `meeting.delete` of a row whose dir was rm'd out
///    of band still works).
/// 2. **Canonical** prefix match if the file exists. Resolves any
///    symlinks. Catches the case where `dir_path` IS under
///    `meetings/` but is a symlink to somewhere outside.
///
/// Returns the original `PathBuf` (for use as the destination) on
/// success. The wire-side error is opaque (`Internal`); the operator
/// gets the full path detail via `tracing::error!`.
fn validate_meeting_dir(data_dir: &std::path::Path, dir_path: &str) -> Result<PathBuf, IpcError> {
    let dir = PathBuf::from(dir_path);
    let allowed_root = data_dir.join("meetings");

    if !dir.starts_with(&allowed_root) {
        tracing::error!(
            dir = %dir_path,
            allowed = ?allowed_root,
            "registry dir_path escapes data_dir/meetings (lexical check failed)"
        );
        return Err(IpcError::Internal {
            message: "registry path invalid".into(),
        });
    }

    if dir.exists() {
        let canonical_dir = std::fs::canonicalize(&dir).map_err(|e| {
            tracing::error!(error = %e, path = %dir_path, "canonicalize dir_path failed");
            IpcError::Internal {
                message: "registry path invalid".into(),
            }
        })?;
        // `meetings/` may not exist yet; fall back to the lexical root
        // for the canonical comparison if so.
        let canonical_root = std::fs::canonicalize(&allowed_root).unwrap_or(allowed_root);
        if !canonical_dir.starts_with(&canonical_root) {
            tracing::error!(
                canonical_dir = ?canonical_dir,
                canonical_root = ?canonical_root,
                "registry dir_path escapes data_dir/meetings (symlink escape)"
            );
            return Err(IpcError::Internal {
                message: "registry path invalid".into(),
            });
        }
    }

    Ok(dir)
}

/// Convert a `panops_core::storage::Meeting` (domain) to a
/// `panops_protocol::Meeting` (wire). Same field set today; the
/// explicit map keeps domain free to evolve without breaking wire.
fn to_protocol_meeting(m: panops_core::storage::Meeting) -> Meeting {
    Meeting {
        id: m.id,
        title: m.title,
        started_at: m.started_at,
        ended_at: m.ended_at,
        duration_ms: m.duration_ms,
        language: m.language,
        dir_path: m.dir_path,
    }
}

/// Run a synchronous closure on the blocking pool and convert the
/// result into a JSON-RPC `ErrorObjectOwned`. Centralizes the
/// `JoinError -> Internal` mapping so each handler stays focused on
/// its own logic. `op` is the method name for tracing context.
async fn spawn_blocking_into_ipc<T, F>(op: &'static str, f: F) -> Result<T, ErrorObjectOwned>
where
    F: FnOnce() -> Result<T, IpcError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| {
            tracing::error!(method = op, error = %e, "spawn_blocking join error");
            ipc_error_to_obj(IpcError::Internal {
                message: format!("{op} internal error"),
            })
        })?
        .map_err(ipc_error_to_obj)
}

#[cfg(test)]
mod readiness_tests {
    //! Tests for the warmup gate added by #74 (eager-after-bind).
    //!
    //! These exercise `run_notes_pipeline` directly because the gate
    //! is a synchronous early-return — no need to spin up the full
    //! jsonrpsee server. Integration tests use `EngineServices::ready`
    //! which pre-fills the `OnceLock`, so they don't see this path.

    use super::*;
    use panops_core::conformance::fakes::MockLlm;
    use panops_core::llm::LlmProvider;

    fn dummy_params() -> NotesGenerateParams {
        NotesGenerateParams {
            audio: "/tmp/whatever.wav".into(),
            dialect: None,
            llm_provider: None,
            llm_model: None,
            no_diarize: None,
            language: None,
            meeting_id: None,
        }
    }

    #[test]
    fn pending_services_yield_provider_unavailable_during_warmup() {
        let llm: Arc<dyn LlmProvider + Send + Sync> = Arc::new(MockLlm::default());
        let storage: Arc<dyn panops_core::storage::Storage> =
            Arc::new(panops_core::conformance::fakes::InMemoryStorage::new());
        let (services, _heavy_lock) =
            crate::server::EngineServices::pending(llm, storage, std::path::PathBuf::from("/tmp"));
        let (events_tx, _) = broadcast::channel(16);
        let err = run_notes_pipeline(&services, &dummy_params(), &events_tx, "test-job")
            .expect_err("warmup must error");
        match err {
            IpcError::ProviderUnavailable { message } => {
                assert!(
                    message.contains("warming up"),
                    "expected warming-up message, got: {message}"
                );
            }
            other => panic!("expected ProviderUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn pending_services_yield_internal_when_init_failed() {
        let llm: Arc<dyn LlmProvider + Send + Sync> = Arc::new(MockLlm::default());
        let storage: Arc<dyn panops_core::storage::Storage> =
            Arc::new(panops_core::conformance::fakes::InMemoryStorage::new());
        let (services, heavy_lock) =
            crate::server::EngineServices::pending(llm, storage, std::path::PathBuf::from("/tmp"));
        // Simulate init failure (e.g., model download blew up).
        heavy_lock
            .set(Err("simulated whisper init failure".to_string()))
            .map_err(|_| ())
            .expect("set OnceLock");
        let (events_tx, _) = broadcast::channel(16);
        let err = run_notes_pipeline(&services, &dummy_params(), &events_tx, "test-job")
            .expect_err("init failure must surface");
        match err {
            IpcError::Internal { message } => {
                assert!(
                    message.contains("adapter init failed"),
                    "expected init-failed prefix, got: {message}"
                );
                assert!(
                    message.contains("simulated whisper init failure"),
                    "expected wrapped error, got: {message}"
                );
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod notes_json_sidecar_tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};
    use panops_core::notes::ir::{NotesFrontmatter, NotesSection};

    fn sample_notes() -> StructuredNotes {
        StructuredNotes {
            schema_version: StructuredNotes::SCHEMA_VERSION,
            frontmatter: NotesFrontmatter {
                title: "Sidebar IR test".into(),
                date: NaiveDate::from_ymd_opt(2026, 6, 7).unwrap(),
                started_at: chrono::FixedOffset::east_opt(0)
                    .unwrap()
                    .with_ymd_and_hms(2026, 6, 7, 12, 0, 0)
                    .unwrap(),
                duration_ms: 60_000,
                speakers: vec!["speaker_0".into()],
                languages: vec!["en".into()],
                tags: vec!["test".into()],
                template: "default".into(),
                dialect: MarkdownDialect::Basic,
                panops_version: "0.1.0".into(),
                source_audio: None,
            },
            sections: vec![NotesSection {
                index: 1,
                title: "Summary".into(),
                time_range_ms: (0, 60_000),
                narrative_md: "The generated sidecar round-trips.".into(),
                key_points: vec!["Structured notes are preserved".into()],
                action_items: Vec::new(),
                screenshots: Vec::new(),
            }],
            language: "en".into(),
            generated_at: Utc.with_ymd_and_hms(2026, 6, 7, 12, 1, 0).unwrap(),
        }
    }

    #[test]
    fn write_structured_notes_json_creates_round_trippable_sidecar() {
        let dir = tempfile::tempdir().expect("tempdir");
        let notes = sample_notes();

        let path = write_structured_notes_json(&notes, dir.path()).expect("write notes.json");

        assert_eq!(path, dir.path().join("notes.json"));
        assert!(path.exists(), "notes.json should exist at {path:?}");
        let body = std::fs::read_to_string(&path).expect("read notes.json");
        let back: StructuredNotes =
            serde_json::from_str(&body).expect("notes.json parses as StructuredNotes");
        assert_eq!(back, notes);
    }
}

#[cfg(test)]
mod notes_generate_concurrency_tests {
    use super::{IpcImpl, IpcServer};
    use crate::server::MAX_CONCURRENT_NOTES_JOBS;
    use panops_core::conformance::fakes::{
        FakeNotesExporter, InMemoryStorage, KnownTurnsFake, TranscriptFileFake,
    };
    use panops_core::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};
    use panops_core::vad::{SpeechRegion, Vad, VadError};
    use panops_protocol::{Event, NotesDialect, NotesGenerateParams};
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;
    use tokio::sync::broadcast;

    struct ConcurrencyProbe {
        in_flight: AtomicUsize,
        max_seen: AtomicUsize,
        section_calls_started: AtomicUsize,
        released: Mutex<bool>,
        wait_cv: Condvar,
    }

    impl ConcurrencyProbe {
        fn new() -> Self {
            Self {
                in_flight: AtomicUsize::new(0),
                max_seen: AtomicUsize::new(0),
                section_calls_started: AtomicUsize::new(0),
                released: Mutex::new(false),
                wait_cv: Condvar::new(),
            }
        }

        fn enter_blocking_section_call(&self) {
            self.section_calls_started.fetch_add(1, Ordering::SeqCst);
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_seen.fetch_max(now, Ordering::SeqCst);

            let mut released = self.released.lock().expect("probe mutex poisoned");
            while !*released {
                released = self.wait_cv.wait(released).expect("probe condvar poisoned");
            }

            self.in_flight.fetch_sub(1, Ordering::SeqCst);
        }

        fn release_all(&self) {
            *self.released.lock().expect("probe mutex poisoned") = true;
            self.wait_cv.notify_all();
        }

        async fn wait_for_started(&self, expected: usize) {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if self.section_calls_started.load(Ordering::SeqCst) >= expected {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("timed out waiting for bounded jobs to enter LLM");
        }
    }

    struct BlockingSectionLlm {
        probe: Arc<ConcurrencyProbe>,
    }

    impl LlmProvider for BlockingSectionLlm {
        fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
            let system = req.system.as_deref().unwrap_or_default();
            if system.contains("meeting-notes writer") {
                self.probe.enter_blocking_section_call();
                return Ok(LlmResponse::Json(serde_json::json!({
                    "title": "Bounded notes job",
                    "narrative_md": "The meeting was summarized without exceeding the concurrency bound.",
                    "key_points": ["Concurrency stayed bounded"],
                    "action_items": [{"description": "Keep job backpressure in place", "owner": null}]
                })));
            }

            if system.contains("meeting-notes editor") {
                return Ok(LlmResponse::Json(serde_json::json!({
                    "title": "Bounded notes jobs",
                    "tags": ["bounded-concurrency"]
                })));
            }

            Err(LlmError::Provider(format!(
                "unexpected prompt system: {system:?}"
            )))
        }
    }

    struct AllSpeechVad;

    impl Vad for AllSpeechVad {
        fn detect_speech(
            &self,
            samples: &[f32],
            sample_rate: u32,
        ) -> Result<Vec<SpeechRegion>, VadError> {
            Ok(vec![SpeechRegion {
                start_ms: 0,
                end_ms: (samples.len() as u64 * 1000) / u64::from(sample_rate),
            }])
        }
    }

    fn notes_params(audio_path: &Path) -> NotesGenerateParams {
        NotesGenerateParams {
            audio: audio_path.to_string_lossy().into_owned(),
            dialect: Some(NotesDialect::Basic),
            llm_provider: None,
            llm_model: None,
            no_diarize: Some(true),
            language: Some("en".into()),
            meeting_id: None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn notes_generate_runs_no_more_than_max_concurrent_jobs() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root above crates/panops-engine");
        let audio_path = repo_root
            .join("tests")
            .join("fixtures")
            .join("audio")
            .join("multi_speaker_60s.wav");
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let probe = Arc::new(ConcurrencyProbe::new());
        let services = crate::server::EngineServices::ready(
            Arc::new(BlockingSectionLlm {
                probe: probe.clone(),
            }),
            Arc::new(InMemoryStorage::new()),
            data_dir.path().to_path_buf(),
            Arc::new(TranscriptFileFake::from_text(
                "The team reviewed bounded job concurrency.",
                Some("en"),
            )),
            Arc::new(KnownTurnsFake),
            Arc::new(FakeNotesExporter),
            Arc::new(AllSpeechVad),
        );
        let (events_tx, mut events_rx) = broadcast::channel(16);
        let ipc = Arc::new(IpcImpl {
            services: Arc::new(services),
            events_tx,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            auto_notes: Arc::new(Mutex::new(HashMap::new())),
        });

        let total_jobs = MAX_CONCURRENT_NOTES_JOBS + 2;
        let mut rpc_handles = Vec::with_capacity(total_jobs);
        for _ in 0..total_jobs {
            let ipc = ipc.clone();
            let params = notes_params(&audio_path);
            rpc_handles.push(tokio::spawn(
                async move { ipc.notes_generate(params).await },
            ));
        }

        probe.wait_for_started(MAX_CONCURRENT_NOTES_JOBS).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            probe.section_calls_started.load(Ordering::SeqCst),
            MAX_CONCURRENT_NOTES_JOBS,
            "jobs beyond the semaphore bound should not enter the pipeline while permits are held",
        );
        assert_eq!(
            probe.max_seen.load(Ordering::SeqCst),
            MAX_CONCURRENT_NOTES_JOBS,
            "more notes jobs ran concurrently than the configured semaphore bound",
        );

        probe.release_all();

        for handle in rpc_handles {
            let accepted = tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .expect("notes.generate RPC task should return")
                .expect("notes.generate RPC task should not panic")
                .expect("notes.generate should accept bounded job");
            assert!(!accepted.job_id.is_empty());
        }

        let mut done = 0;
        tokio::time::timeout(Duration::from_secs(10), async {
            while done < total_jobs {
                match events_rx.recv().await.expect("events channel open") {
                    Event::JobDone(_) => done += 1,
                    Event::JobError(e) => panic!("notes job errored: {:?}", e.error),
                    Event::Unknown(v) => panic!("unexpected unknown event: {v}"),
                    Event::Screenshot(_) | Event::RecordingProgress(_) | Event::JobProgress(_) => {}
                }
            }
        })
        .await
        .expect("all accepted notes jobs should finish");
    }
}

#[cfg(test)]
mod recording_auto_generate_tests {
    use super::*;

    use panops_core::conformance::fakes::{InMemoryStorage, MockLlm};
    use std::time::Duration;

    static TEST_CAPTURE_INIT: std::sync::Once = std::sync::Once::new();

    fn ensure_test_capture() {
        TEST_CAPTURE_INIT.call_once(|| {
            // SAFETY: unit tests set this before the capture resolver is first
            // used in this process.
            unsafe {
                std::env::set_var("PANOPS_TEST_CAPTURE", "1");
            }
        });
    }

    fn pending_ipc() -> (IpcImpl, tempfile::TempDir) {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let storage = Arc::new(InMemoryStorage::new());
        let (services, _heavy) = crate::server::EngineServices::pending(
            Arc::new(MockLlm::default()),
            storage,
            data_dir.path().to_path_buf(),
        );
        let (events_tx, _keepalive) = broadcast::channel(16);
        (
            IpcImpl {
                services: Arc::new(services),
                events_tx,
                sessions: Arc::new(Mutex::new(HashMap::new())),
                auto_notes: Arc::new(Mutex::new(HashMap::new())),
            },
            data_dir,
        )
    }

    #[tokio::test]
    async fn recording_start_stores_auto_generate_notes_engine_side() {
        ensure_test_capture();
        let (ipc, _data_dir) = pending_ipc();
        let meeting_id = ipc
            .meeting_start(MeetingConfig {
                title: Some("auto flag session".into()),
                language: Some("en".into()),
            })
            .await
            .expect("meeting.start");

        ipc.recording_start(RecordingStartParams {
            meeting_id: meeting_id.clone(),
            audio_sources: panops_protocol::AudioSourcesWire::SystemAndMic,
            record_video: false,
            auto_generate_notes: true,
            screenshot_interval_ms: 500,
            screenshot_threshold: 0.15,
            capture_target: panops_protocol::CaptureTarget::Display { display_id: 0 },
            width: None,
            height: None,
        })
        .await
        .expect("recording.start");

        assert!(
            ipc.sessions
                .lock()
                .expect("sessions lock")
                .contains_key(&meeting_id),
            "recording.start must still store the capture session"
        );
        let auto_generate_notes = ipc
            .auto_notes
            .lock()
            .expect("auto_notes lock")
            .get(&meeting_id)
            .copied()
            .expect("auto-generate flag stored engine-side");
        assert!(
            auto_generate_notes,
            "recording.start must store auto_generate_notes in engine-side session state"
        );

        let _ = ipc
            .recording_stop(RecordingStopParams {
                recording_id: meeting_id.clone(),
            })
            .await
            .expect("recording.stop cleanup");

        assert!(
            !ipc.auto_notes
                .lock()
                .expect("auto_notes lock")
                .contains_key(&meeting_id),
            "recording.stop must remove engine-side auto-generate state"
        );
        assert!(
            !ipc.sessions
                .lock()
                .expect("sessions lock")
                .contains_key(&meeting_id),
            "recording.stop must remove capture session state"
        );
    }

    #[tokio::test]
    async fn recording_stop_auto_generate_skips_when_compute_is_not_ready() {
        ensure_test_capture();
        let (ipc, _data_dir) = pending_ipc();
        let mut rx = ipc.events_tx.subscribe();
        let meeting_id = ipc
            .meeting_start(MeetingConfig {
                title: Some("deferred auto notes".into()),
                language: Some("en".into()),
            })
            .await
            .expect("meeting.start");

        ipc.recording_start(RecordingStartParams {
            meeting_id: meeting_id.clone(),
            audio_sources: panops_protocol::AudioSourcesWire::SystemAndMic,
            record_video: false,
            auto_generate_notes: true,
            screenshot_interval_ms: 500,
            screenshot_threshold: 0.15,
            capture_target: panops_protocol::CaptureTarget::Display { display_id: 0 },
            width: None,
            height: None,
        })
        .await
        .expect("recording.start");

        let stopped = ipc
            .recording_stop(RecordingStopParams {
                recording_id: meeting_id,
            })
            .await
            .expect("recording.stop should still succeed");
        assert!(stopped.system_audio_path.is_some() || stopped.mic_audio_path.is_some());
        assert!(
            stopped.notes_job_id.is_none(),
            "no notes job id when compute is unavailable (app shows a deferred hint)"
        );

        let no_event = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        assert!(
            no_event.is_err(),
            "auto-generate should not enqueue a notes job while compute is unavailable"
        );
    }

    fn ready_ipc() -> (IpcImpl, tempfile::TempDir) {
        use panops_core::conformance::fakes::{
            FakeNotesExporter, KnownRegionsFake, KnownTurnsFake, TranscriptFileFake,
        };
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let storage = Arc::new(InMemoryStorage::new());
        let services = crate::server::EngineServices::ready(
            Arc::new(MockLlm::default()),
            storage,
            data_dir.path().to_path_buf(),
            Arc::new(TranscriptFileFake::default()),
            Arc::new(KnownTurnsFake),
            Arc::new(FakeNotesExporter),
            Arc::new(KnownRegionsFake::default()),
        );
        let (events_tx, _keepalive) = broadcast::channel(16);
        (
            IpcImpl {
                services: Arc::new(services),
                events_tx,
                sessions: Arc::new(Mutex::new(HashMap::new())),
                auto_notes: Arc::new(Mutex::new(HashMap::new())),
            },
            data_dir,
        )
    }

    #[tokio::test]
    async fn recording_stop_auto_generate_returns_job_id_when_ready() {
        ensure_test_capture();
        let (ipc, _data_dir) = ready_ipc();
        let meeting_id = ipc
            .meeting_start(MeetingConfig {
                title: Some("ready auto notes".into()),
                language: Some("en".into()),
            })
            .await
            .expect("meeting.start");

        ipc.recording_start(RecordingStartParams {
            meeting_id: meeting_id.clone(),
            audio_sources: panops_protocol::AudioSourcesWire::SystemAndMic,
            record_video: false,
            auto_generate_notes: true,
            screenshot_interval_ms: 500,
            screenshot_threshold: 0.15,
            capture_target: panops_protocol::CaptureTarget::Display { display_id: 0 },
            width: None,
            height: None,
        })
        .await
        .expect("recording.start");

        let stopped = ipc
            .recording_stop(RecordingStopParams {
                recording_id: meeting_id,
            })
            .await
            .expect("recording.stop");

        assert!(stopped.system_audio_path.is_some() || stopped.mic_audio_path.is_some());
        assert!(
            stopped.notes_job_id.is_some(),
            "auto-generate with a ready provider must surface the enqueued notes job id"
        );
    }
}
