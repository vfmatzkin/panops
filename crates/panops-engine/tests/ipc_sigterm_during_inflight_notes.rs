//! Issue #90 — shutdown while a `notes.generate` pipeline is in-flight.
//!
//! R3 reworked shutdown to a `tokio::sync::watch::channel(bool)`
//! end-to-end (`crates/panops-engine/src/server/mod.rs`). The existing
//! tests only fire shutdown *after* they've already received the awaited
//! `JobDone` — none of them exercise shutdown landing mid-pipeline. A
//! regression that disconnects the watch from the bridge task, or that
//! breaks `stop_handle.shutdown()`'s await contract, would let an
//! in-flight job hang its subscriber forever. This test closes that gap.
//!
//! Why in-process (`run_serve_in_process` + a `watch` sender) rather
//! than spawning the binary and sending a real `SIGTERM`:
//!
//!  1. The subprocess `serve` path *always* loads the real Whisper /
//!     Sherpa / VAD models — `init_heavy_adapters` (server/mod.rs) calls
//!     `ensure_model` / `ensure_diar_models` / `ensure_vad_model`
//!     unconditionally and has no fake hook (`PANOPS_FAKE_ASR` is only
//!     honored in the CLI `transcribe` path, never in `serve`). So a
//!     *deterministic* in-flight `notes.generate` is not reachable from
//!     the binary without model downloads, which `cargo test --locked`
//!     can't depend on. Every deterministic notes-pipeline test in this
//!     crate uses `run_serve_in_process` + fakes for exactly this reason.
//!  2. The real-`SIGTERM` → clean-exit + socket-cleanup contract (with
//!     *no* in-flight job) is already covered by
//!     `ipc_server_starts_and_binds.rs`.
//!  3. Production routes BOTH the OS SIGINT/SIGTERM handler AND the
//!     optional `external_shutdown` receiver into the SAME internal
//!     `watch` channel (`run_serve_in_process`), so firing the watch
//!     drives the identical drain path a real signal triggers
//!     (watch → bridge task → `serve_with_graceful_shutdown`), minus the
//!     OS-signal → watch hop that (2) already proves.
//!
//! What's asserted, deterministically:
//!  - `run_serve_in_process` returns `Ok(())` within a few seconds of
//!    shutdown — the accept loop breaks and cleans up; it never hangs.
//!  - The in-flight events subscription drains to a clean close within
//!    budget, never timing out. Per the issue, which side of the race
//!    wins is unspecified ("either `JobDone` drained, or cancelled"), so
//!    any terminal event is accepted on the way down; the load-bearing
//!    assertion is that the connection is reaped and nothing hangs.
//!
//! Once cancellation tokens land (#81), this can tighten to assert a
//! `JobError { Cancelled }` specifically.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::{ClientT, Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use panops_core::Transcript;
use panops_core::asr::{AsrError, AsrProvider};
use panops_core::notes::dialect::MarkdownDialect;
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_portable::markdown_exporter::MarkdownExporter;
use panops_protocol::{Event, JobAccepted};
use tempfile::tempdir;
use tokio::sync::{Notify, watch};

use common::notes_pipeline::{
    DeterministicAsr, DeterministicDiar, SingleRegionVad, build_mock_llm,
};
use common::{tempdir_storage, uds_ws_client, wait_for_socket};

/// ASR fake that blocks for `delay` on every `transcribe`, pinging
/// `entered` the instant it starts. The test waits on `entered` before
/// firing shutdown, so the pipeline is *provably* mid-flight (no racing
/// a fixed sleep). Delegates to `DeterministicAsr` for the golden
/// transcript so the downstream `MockLlm` fingerprints still match and a
/// clean `JobDone` stays reachable if shutdown loses the race.
struct SlowAsr {
    entered: Arc<Notify>,
    delay: Duration,
}

impl AsrProvider for SlowAsr {
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        self.entered.notify_one();
        std::thread::sleep(self.delay);
        DeterministicAsr.transcribe(samples, sample_rate, language_hint)
    }

    // Must stay a fake so `transcribe_recursive`'s duration trigger
    // (gated on `!asr.is_fake()`) doesn't split the region and duplicate
    // the golden segments out from under the MockLlm fingerprints.
    fn is_fake(&self) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_during_inflight_notes_drains_without_hanging() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let audio_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-engine")
        .join("tests/fixtures/audio/multi_speaker_60s.wav");

    let (_storage_tmp, storage, data_dir) = tempdir_storage();

    // ASR sleeps long enough to still be running when graceful shutdown
    // begins draining; `entered` removes the timing guesswork.
    let entered = Arc::new(Notify::new());
    let services = EngineServices::ready(
        Arc::new(build_mock_llm(MarkdownDialect::Basic)),
        storage,
        data_dir,
        Arc::new(SlowAsr {
            entered: entered.clone(),
            delay: Duration::from_millis(1500),
        }),
        Arc::new(DeterministicDiar),
        Arc::new(MarkdownExporter),
        Arc::new(SingleRegionVad),
    );

    let server_socket = socket.clone();
    let server = tokio::spawn(async move {
        run_serve_in_process(&server_socket, services, Some(shutdown_rx)).await
    });

    wait_for_socket(&socket).await;
    let client = uds_ws_client(&socket).await;

    // Subscribe FIRST so we never race the broadcast.
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

    // Block until the pipeline is genuinely inside the ASR stage, THEN
    // signal shutdown — this is the mid-pipeline shutdown the issue wants.
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("ASR stage entered within 5s");
    let _ = shutdown_tx.send(true);

    // (1) The server future returns Ok within budget — no hang.
    let joined = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server future resolved within 5s");
    let server_res = joined.expect("server task joined cleanly (no panic)");
    assert!(
        server_res.is_ok(),
        "run_serve_in_process errored on shutdown: {server_res:?}"
    );

    // (2) The in-flight subscription drains to a clean close within
    //     budget, never timing out. Each `next()` is bounded: a hang here
    //     is exactly the regression this test guards against (watch
    //     disconnected from the bridge, or the connection never reaped).
    loop {
        let next = tokio::time::timeout(Duration::from_secs(5), subscription.next())
            .await
            .expect("subscription drained without hanging");
        match next {
            // Drained terminal events are fine (the issue leaves the
            // drain-vs-cancel race unspecified); keep going until close.
            Some(Ok(Event::JobDone(_))) | Some(Ok(Event::JobError(_))) => continue,
            // Ignore non-terminal noise (screenshot / progress events
            // that may arrive from concurrent tests) and Unknown.
            Some(Ok(_)) => continue,
            // Transport/deserialise error or a clean close: shutdown
            // reaped the connection. Either way, nothing hung.
            Some(Err(_)) | None => break,
        }
    }
}
