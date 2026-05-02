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
    Event, IpcError, JobAccepted, JobDoneEvent, JobErrorEvent, MeetingSummary, NotesDialect,
    NotesGenerateParams, NotesGenerateResult,
};
use tokio::sync::broadcast;

#[rpc(server, namespace = "ipc", namespace_separator = ".")]
pub(super) trait Ipc {
    #[method(name = "notes.generate")]
    async fn notes_generate(
        &self,
        params: NotesGenerateParams,
    ) -> Result<JobAccepted, ErrorObjectOwned>;

    #[method(name = "meeting.list")]
    async fn meeting_list(&self) -> Result<Vec<MeetingSummary>, ErrorObjectOwned>;

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
        // Slice 05 stub. Backed by SQLite once #17 lands; ships now to
        // lock the response shape (see spec §D9).
        Ok(Vec::new())
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
fn run_notes_pipeline(
    services: &crate::server::EngineServices,
    params: &NotesGenerateParams,
) -> Result<NotesGenerateResult, IpcError> {
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

    let mut transcript = services
        .asr
        .transcribe_full(&audio_path, params.language.as_deref())
        .map_err(IpcError::from)?;

    let no_diarize = params.no_diarize.unwrap_or(false);
    if !no_diarize {
        let turns = services.diar.diarize(&audio_path).map_err(IpcError::from)?;
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

    // Output dir convention matches CLI `run_notes`: `<audio_stem>-notes`
    // alongside the audio file. Falls back to `./notes` if the parent
    // can't be resolved (shouldn't happen for valid inputs but keeps
    // unwrap-free).
    let stem = audio_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "notes".to_string());
    let out_dir = audio_path
        .parent()
        .map(|p| p.join(format!("{stem}-notes")))
        .unwrap_or_else(|| PathBuf::from("./notes"));
    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            // Wire-side message stays opaque: the full path + os error
            // would let a probing client map the local FS layout. The
            // operator gets the detail via tracing.
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

    let artifact = services.exporter.export(&notes, &out_dir).map_err(|e| {
        // Domain-to-wire mapping lives in `panops-protocol` (gated by
        // `domain-conversions`); we still log the full error here so
        // the operator sees template / FS detail that the wire-side
        // message intentionally hides.
        tracing::error!(error = %e, "notes.generate exporter failed");
        IpcError::from(e)
    })?;

    Ok(NotesGenerateResult {
        primary_file: artifact.primary_file.display().to_string(),
        assets: artifact
            .assets
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
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
#[allow(dead_code)]
pub(super) fn ipc_error_to_obj(e: IpcError) -> ErrorObjectOwned {
    let data = serde_json::to_value(&e).expect("IpcError serialise");
    ErrorObjectOwned::owned(-32000, e.to_string(), Some(data))
}
