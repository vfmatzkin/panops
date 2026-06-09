//! Slice 11 — two-track live-capture pipeline. When a meeting dir holds
//! `system.wav` (remote participants) + `mic.wav` (the local user),
//! `notes.generate` pins the mic track to local speaker 0 ("You") and
//! diarizes only the system track for remote speakers (ids >= 1),
//! merging both into one timestamp-ordered transcript.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::{ClientT, Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use panops_core::Transcript;
use panops_core::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_portable::markdown_exporter::MarkdownExporter;
use panops_protocol::{Event, JobAccepted};
use tempfile::tempdir;
use tokio::sync::watch;

use common::notes_pipeline::{DeterministicAsr, DeterministicDiar, SingleRegionVad};
use common::{tempdir_storage, uds_ws_client, wait_for_socket};

/// Permissive LLM stub: the two-track merged transcript produces a prompt
/// the deterministic `MockLlm` doesn't have a canned response for. This
/// test exercises the *attribution* stage (transcript.json), not prompt
/// fidelity, so it returns one valid section/frontmatter JSON for every
/// call, letting the pipeline reach `JobDone`.
struct PermissiveLlm;

impl LlmProvider for PermissiveLlm {
    fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse::Json(serde_json::json!({
            "title": "Two-track meeting",
            "narrative_md": "A discussion took place between the local user and a remote participant.",
            "key_points": [],
            "action_items": [],
            "tags": []
        })))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_track_capture_splits_local_and_remote_speakers() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let fixture_wav = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-engine")
        .join("tests/fixtures/audio/multi_speaker_60s.wav");

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(PermissiveLlm),
        Arc::clone(&storage),
        data_dir,
        Arc::new(DeterministicAsr),
        Arc::new(DeterministicDiar),
        Arc::new(MarkdownExporter),
        Arc::new(SingleRegionVad),
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

    // Create the meeting, then drop two capture tracks into its dir so
    // notes.generate takes the two-track branch (DeterministicAsr returns
    // the same golden segments for each track regardless of WAV content).
    let meeting_id: String = ClientT::request(
        &client,
        "ipc.meeting.start",
        rpc_params![serde_json::json!({"title":"Live capture meeting"})],
    )
    .await
    .expect("meeting.start");

    let meeting_dir = std::path::PathBuf::from(storage.get_meeting(&meeting_id).unwrap().dir_path);
    std::fs::copy(&fixture_wav, meeting_dir.join("system.wav")).expect("write system.wav");
    std::fs::copy(&fixture_wav, meeting_dir.join("mic.wav")).expect("write mic.wav");

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
            "audio": fixture_wav.to_string_lossy(),
            "dialect": "basic",
            "meeting_id": meeting_id.clone(),
        })],
    )
    .await
    .expect("notes.generate");

    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let event = subscription
                .next()
                .await
                .expect("subscription open")
                .expect("payload deserialises");
            match event {
                Event::JobDone(_) => return,
                Event::JobError(e) => panic!("expected JobDone, got JobError: {:?}", e.error),
                Event::Unknown(v) => panic!("expected JobDone, got Unknown: {v}"),
                // Other events may arrive from concurrent tests; ignore them.
                Event::JobProgress(_) => {
                    continue;
                }
            }
        }
    })
    .await
    .expect("event within 60s");

    // The two-track branch writes transcript.json into the meeting dir.
    let transcript_json =
        std::fs::read_to_string(meeting_dir.join("transcript.json")).expect("read transcript.json");
    let transcript: Transcript = serde_json::from_str(&transcript_json).expect("parse transcript");

    assert!(transcript.diarized, "two-track output is diarized");
    assert!(
        transcript.segments.iter().any(|s| s.speaker_id == Some(0)),
        "mic track contributes local speaker 0 segments"
    );
    assert!(
        transcript
            .segments
            .iter()
            .any(|s| matches!(s.speaker_id, Some(id) if id >= 1)),
        "system track contributes remote speaker (>= 1) segments"
    );
    // Merged output stays timestamp-ordered.
    let starts: Vec<u64> = transcript.segments.iter().map(|s| s.start_ms).collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable();
    assert_eq!(starts, sorted, "segments are ordered by start_ms");

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
