//! #89 — a panic inside the notes pipeline must surface as
//! `Event::JobError` on `events.subscribe`, not hang the subscriber or
//! crash the server.
//!
//! The pipeline runs on `spawn_blocking`; the post-spawn awaiter in
//! `handlers::notes_generate` turns the resulting `JoinError` panic into
//! a synthetic `JobError` with an opaque `IpcError::Internal` message so
//! the wire never leaks panic payloads. This test injects a VAD that
//! panics on first call — the earliest pipeline stage after the WAV
//! loads — and asserts the broadcast lands.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::{ClientT, Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_core::vad::{SpeechRegion, Vad, VadError};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::{Event, IpcError, JobAccepted};
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

/// VAD fake that panics on first call. `detect_speech` runs inside the
/// blocking pipeline task right after the WAV loads, so this unwinds
/// before any later stage — exercising the panic -> `JobError` awaiter.
struct PanickingVad;

impl Vad for PanickingVad {
    fn detect_speech(
        &self,
        _samples: &[f32],
        _sample_rate: u32,
    ) -> Result<Vec<SpeechRegion>, VadError> {
        panic!("injected pipeline panic for #89");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pipeline_panic_emits_job_error_internal() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // A real WAV so the pipeline gets past canonicalize + load before the
    // injected VAD panics; the audio content is irrelevant.
    let audio_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-engine")
        .join("tests/fixtures/audio/en_30s.wav");

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    // Every adapter past the VAD is a harmless fake — the panic fires at
    // VAD, so ASR / diar / LLM / exporter are never reached.
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage,
        data_dir,
        Arc::new(TranscriptFileFake::default()),
        Arc::new(KnownTurnsFake),
        Arc::new(FakeNotesExporter),
        Arc::new(PanickingVad),
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

    // The server must catch the panic and broadcast a JobError — not hang
    // (timeout fires) or crash (subscription closes).
    let err = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let ev = subscription
                .next()
                .await
                .expect("subscription stays open after pipeline panic")
                .expect("payload deserialises");
            match ev {
                Event::JobError(err) => return err,
                Event::JobDone(d) => panic!("expected JobError, got JobDone: {:?}", d),
                Event::Unknown(v) => panic!("expected JobError, got Unknown: {v}"),
                // Slice 11 adds Screenshot and RecordingProgress events; ignore them
                // in this notes pipeline test (they may arrive from concurrent tests).
                Event::Screenshot(_) | Event::RecordingProgress(_) | Event::JobProgress(_) => {
                    continue;
                }
            }
        }
    })
    .await
    .expect("JobError arrived within 10s (no hang)");

    // The awaiter maps a panicking JoinError to an opaque Internal error
    // ("pipeline panicked") so no panic payload or path leaks to the wire.
    match err.error {
        IpcError::Internal { message } => assert!(
            message.contains("panic"),
            "expected a panic-derived Internal message, got {message:?}"
        ),
        other => panic!("expected IpcError::Internal from a panic, got {other:?}"),
    }

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
