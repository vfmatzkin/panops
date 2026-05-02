//! Slice 05 — `notes.generate` round-trips through UDS+WS, completes
//! the pipeline on the blocking pool, and surfaces `Event::JobDone`
//! on `events.subscribe`.
//!
//! ASR/diar are deterministic inline fakes (not the sidecar-file fakes
//! in `panops-core::conformance::fakes`) because the slice-04 golden
//! prompt fingerprints depend on the EXACT 3-segment shape used in
//! `regenerate_multi_speaker_60s_goldens`. `TranscriptFileFake` would
//! return one segment covering the whole audio, which mismatches the
//! `MockLlm` fingerprint and panics. The inline fakes echo the same
//! segments / turns the regen test uses, so the section + frontmatter
//! prompts hash identically.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::{ClientT, Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use panops_core::asr::{AsrError, AsrProvider};
use panops_core::conformance::fakes::MockLlm;
use panops_core::diar::{DiarError, Diarizer, SpeakerTurn};
use panops_core::llm::LlmResponse;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::prompts::{
    SectionSummary, build_frontmatter_prompt, build_section_narrative_prompt,
};
use panops_core::{Segment, Transcript};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_portable::markdown_exporter::MarkdownExporter;
use panops_protocol::{Event, JobAccepted};
use tempfile::tempdir;
use tokio::sync::watch;

use common::{uds_ws_client, wait_for_socket};

/// Returns the same 3 segments the slice-04 golden regen uses, so the
/// `MockLlm` prompt fingerprint matches verbatim.
fn golden_segments() -> Vec<Segment> {
    vec![
        Segment {
            start_ms: 0,
            end_ms: 20_000,
            text: "Welcome to this meeting. Let's go over the agenda for today. \
                   We have several important items to discuss in the next sixty minutes together."
                .into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            is_partial: false,
            speaker_id: Some(0),
        },
        Segment {
            start_ms: 20_000,
            end_ms: 40_000,
            text: "Thanks for the introduction. The first item is the budget review for next quarter. \
                   We need to approve the spending plan before the end of this week."
                .into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            is_partial: false,
            speaker_id: Some(1),
        },
        Segment {
            start_ms: 40_000,
            end_ms: 60_000,
            text: "Right. I'll start with the marketing line items, then move to engineering, \
                   and finally we will cover any remaining operations expenses for the team."
                .into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            is_partial: false,
            speaker_id: Some(0),
        },
    ]
}

/// Inline ASR fake that returns the golden segments verbatim. Speaker
/// IDs are already set, so the diar merge below is effectively a no-op
/// (turns map 1:1 to speakers already in the segments).
struct DeterministicAsr;

impl AsrProvider for DeterministicAsr {
    fn transcribe_full(
        &self,
        audio_path: &Path,
        _language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        Ok(Transcript {
            schema_version: Transcript::SCHEMA_VERSION,
            model: "deterministic-asr".into(),
            audio_path: audio_path.to_path_buf(),
            audio_duration_ms: 60_000,
            diarized: false,
            segments: golden_segments(),
        })
    }

    fn is_fake(&self) -> bool {
        true
    }
}

/// Inline diar fake matching `multi_speaker_60s.turns.json`. Returns
/// turns aligned exactly with the segment boundaries above so
/// `merge_speaker_turns` doesn't reshape segments.
struct DeterministicDiar;

impl Diarizer for DeterministicDiar {
    fn diarize(&self, _audio_path: &Path) -> Result<Vec<SpeakerTurn>, DiarError> {
        Ok(vec![
            SpeakerTurn {
                start_ms: 0,
                end_ms: 20_000,
                speaker_id: 0,
            },
            SpeakerTurn {
                start_ms: 20_000,
                end_ms: 40_000,
                speaker_id: 1,
            },
            SpeakerTurn {
                start_ms: 40_000,
                end_ms: 60_000,
                speaker_id: 0,
            },
        ])
    }

    fn is_fake(&self) -> bool {
        true
    }
}

fn build_mock_llm(dialect: MarkdownDialect) -> MockLlm {
    let segments = golden_segments();
    // Match the regen test's canned section / frontmatter responses.
    let canned_section = serde_json::json!({
        "title": "Meeting kickoff and quarterly budget review",
        "narrative_md": "The session opened with a welcome and a brief handoff \
            into the agenda. The first agenda item framed the rest of the \
            meeting, with the discussion organising the review into a clear \
            sequence so each functional area would get its own slot.",
        "key_points": [
            "Budget review scoped to next quarter only",
            "Review sequence: marketing, engineering, operations"
        ],
        "action_items": [
            {"description": "Approve quarterly spending plan before end of week", "owner": null}
        ]
    });
    let canned_fm = serde_json::json!({
        "title": "Quarterly budget review kickoff",
        "tags": ["budget-review", "quarterly", "kickoff"]
    });
    let summaries = vec![SectionSummary {
        title: "Meeting kickoff and quarterly budget review".into(),
        key_points: vec![
            "Budget review scoped to next quarter only".into(),
            "Review sequence: marketing, engineering, operations".into(),
        ],
    }];
    let section_prompt = build_section_narrative_prompt(&segments, dialect, "en");
    let frontmatter_prompt = build_frontmatter_prompt(&summaries, "en", 60_000);
    MockLlm::default()
        .with_response_for(
            section_prompt.system.as_deref(),
            &section_prompt.user,
            LlmResponse::Json(canned_section),
        )
        .with_response_for(
            frontmatter_prompt.system.as_deref(),
            &frontmatter_prompt.user,
            LlmResponse::Json(canned_fm),
        )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notes_generate_round_trip_emits_job_done() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Stage the audio path inside a tempdir so the pipeline's output
    // directory (alongside the audio) doesn't pollute repo fixtures.
    // The wav doesn't need real bytes — `DeterministicAsr` ignores
    // file contents.
    let audio_dir = tempdir().unwrap();
    let audio_path = audio_dir.path().join("multi_speaker_60s.wav");
    std::fs::write(&audio_path, b"placeholder").unwrap();

    let services = EngineServices {
        llm: Arc::new(build_mock_llm(MarkdownDialect::Basic)),
        asr: Arc::new(DeterministicAsr),
        diar: Arc::new(DeterministicDiar),
        exporter: Arc::new(MarkdownExporter),
    };

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

    let event = tokio::time::timeout(Duration::from_secs(60), subscription.next())
        .await
        .expect("event arrived within 60s")
        .expect("subscription not closed")
        .expect("event payload deserialised");

    let primary_file = match event {
        Event::JobDone(d) => PathBuf::from(d.result.primary_file),
        Event::JobError(e) => panic!("expected JobDone, got JobError: {:?}", e.error),
    };
    assert!(
        primary_file.exists(),
        "primary_file does not exist: {primary_file:?}"
    );

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
