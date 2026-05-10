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

use std::path::PathBuf;
use std::sync::Arc;

use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::ErrorObjectOwned;
use panops_core::merge::merge_speaker_turns;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::input::{MeetingMetadata, NotesInput};
use panops_core::notes::pipeline::NotesGenerator;
use panops_protocol::{
    Event, IpcError, JobAccepted, JobDoneEvent, JobErrorEvent, Meeting, MeetingConfig,
    MeetingSummary, NotesDialect, NotesGenerateParams, NotesGenerateResult,
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

    #[method(name = "meeting.list")]
    async fn meeting_list(&self) -> Result<Vec<MeetingSummary>, ErrorObjectOwned>;

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

    #[method(name = "meeting.delete")]
    async fn meeting_delete(&self, params: MeetingIdParam) -> Result<(), ErrorObjectOwned>;

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
}

#[async_trait::async_trait]
impl IpcServer for IpcImpl {
    async fn notes_generate(
        &self,
        params: NotesGenerateParams,
    ) -> Result<JobAccepted, ErrorObjectOwned> {
        let job_id = uuid::Uuid::new_v4().to_string();
        let services = self.services.clone();
        let events_tx = self.events_tx.clone();
        let job_id_owned = job_id.clone();

        // Move the pipeline off any tokio worker thread: rayon (used by
        // `NotesGenerator` for the per-section fan-out) and the blocking
        // ASR/diar adapters mustn't share a runtime worker with the RPC
        // accept loop. `spawn_blocking` drops them on the dedicated
        // blocking pool. The `notes.generate` RPC returns immediately;
        // the actual result lands on `events.subscribe` as `JobDone`
        // or `JobError`.
        let job_id_for_panic = job_id.clone();
        let events_tx_for_panic = events_tx.clone();
        let join_handle = tokio::task::spawn_blocking(move || {
            let outcome = run_notes_pipeline(&services, &params);
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
        tokio::spawn(async move {
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

        Ok(JobAccepted { job_id })
    }

    async fn meeting_list(&self) -> Result<Vec<MeetingSummary>, ErrorObjectOwned> {
        let storage = self.services.storage.clone();
        let rows = tokio::task::spawn_blocking(move || storage.list_meetings())
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "meeting.list spawn_blocking join");
                ipc_error_to_obj(IpcError::Internal {
                    message: "meeting.list internal error".into(),
                })
            })?
            .map_err(|e| ipc_error_to_obj(e.into()))?;

        // Convert from `panops_core::storage::MeetingSummary` to the
        // wire-shape `panops_protocol::MeetingSummary`. Same field set
        // today; the explicit map keeps us free to diverge later.
        Ok(rows
            .into_iter()
            .map(|s| MeetingSummary {
                id: s.id,
                title: s.title,
                started_at: s.started_at,
                duration_ms: s.duration_ms,
            })
            .collect())
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

    let mut transcript = heavy
        .asr
        .transcribe_full(&audio_path, params.language.as_deref())
        .map_err(IpcError::from)?;

    let no_diarize = params.no_diarize.unwrap_or(false);
    if !no_diarize {
        let turns = heavy.diar.diarize(&audio_path).map_err(IpcError::from)?;
        transcript.segments = merge_speaker_turns(transcript.segments, &turns);
        transcript.diarized = true;
    }

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
            source_path: Some(audio_path.clone()),
            language_hint: params.language.clone(),
        },
    };

    let generator = NotesGenerator {
        llm: services.llm.as_ref(),
        dialect,
    };
    let notes = generator.generate(input).map_err(IpcError::from)?;
    let exporter = heavy.exporter.clone();

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

    let artifact = exporter.export(&notes, &out_dir).map_err(|e| {
        // Domain-to-wire mapping lives in `panops-protocol` (gated by
        // `domain-conversions`); we still log the full error here so
        // the operator sees template / FS detail that the wire-side
        // message intentionally hides.
        tracing::error!(error = %e, "notes.generate exporter failed");
        IpcError::from(e)
    })?;

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
    })
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
        let err = run_notes_pipeline(&services, &dummy_params()).expect_err("warmup must error");
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
        let err =
            run_notes_pipeline(&services, &dummy_params()).expect_err("init failure must surface");
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
