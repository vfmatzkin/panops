//! Late subscribers to `events.subscribe` do NOT receive prior events.
//!
//! `tokio::sync::broadcast` semantics: a receiver created by `subscribe()`
//! only receives messages sent AFTER that call. This test confirms that
//! contract holds for panops's event channel — a client subscribing after
//! an event was already emitted misses that event.

mod common;

use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::{ClientT, Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use panops_core::conformance::fakes::{
    FakeNotesExporter, KnownRegionsFake, KnownTurnsFake, MockLlm, TranscriptFileFake,
};
use panops_engine::server::{EngineServices, run_serve_in_process};
use panops_protocol::{Event, IpcError, JobAccepted};
use tempfile::tempdir;
use tokio::sync::watch;

use common::{tempdir_storage, uds_ws_client, wait_for_socket};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_subscriber_does_not_receive_prior_events() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (_storage_tmp, storage, data_dir) = tempdir_storage();
    let services = EngineServices::ready(
        Arc::new(MockLlm::default()),
        storage,
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

    // Subscribe EARLY to confirm events are actually emitted.
    let mut early_sub: Subscription<Event> = SubscriptionClientT::subscribe(
        &client,
        "ipc.events.subscribe",
        rpc_params![],
        "ipc.events.unsubscribe",
    )
    .await
    .expect("early subscribe to events");

    // Trigger a job that will emit JobError (nonexistent audio path).
    let _accepted: JobAccepted = ClientT::request(
        &client,
        "ipc.notes.generate",
        rpc_params![serde_json::json!({
            "audio": "/nonexistent/path.wav",
        })],
    )
    .await
    .expect("call notes.generate");

    // Wait for the JobError on the EARLY subscription — confirms the event
    // was actually broadcast before we subscribe late.
    let early_err = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let ev = early_sub
                .next()
                .await
                .expect("subscription open")
                .expect("payload deserialises");
            match ev {
                Event::JobError(err) => return err,
                Event::JobDone(d) => panic!("expected JobError, got JobDone: {:?}", d),
                Event::Unknown(v) => panic!("expected JobError, got Unknown: {v}"),
                // Ignore screenshot/recording progress from concurrent tests.
                Event::JobProgress(_) => {
                    continue;
                }
            }
        }
    })
    .await
    .expect("early subscriber received JobError within 10s");

    assert!(
        matches!(early_err.error, IpcError::InputNotFound { .. }),
        "early subscriber got InputNotFound, proving event was emitted"
    );

    // NOW subscribe LATE — after the event was already broadcast.
    let mut late_sub: Subscription<Event> = SubscriptionClientT::subscribe(
        &client,
        "ipc.events.subscribe",
        rpc_params![],
        "ipc.events.unsubscribe",
    )
    .await
    .expect("late subscribe to events");

    // The late subscriber should NOT receive the prior JobError.
    // broadcast::Receiver only yields messages sent after `subscribe()`.
    // Use a short timeout — if we receive anything, it must be a NEW event
    // (not the prior JobError). Timeout proves no prior event replay.
    let late_result = tokio::time::timeout(Duration::from_millis(500), async {
        late_sub
            .next()
            .await
            .expect("subscription open")
            .expect("payload deserialises")
    })
    .await;

    // Timeout is the expected outcome — no prior event to receive.
    // If we got an event, verify it's NOT the prior JobError (it would be
    // a spurious event from a concurrent test, which we ignore).
    match late_result {
        Err(_timeout) => {} // Expected: no prior event.
        Ok(Event::JobError(err)) => {
            panic!(
                "late subscriber received prior JobError that early subscriber already consumed: {:?}",
                err.error
            );
        }
        Ok(Event::JobDone(d)) => {
            panic!("late subscriber received prior JobDone: {:?}", d);
        }
        Ok(Event::JobProgress(_) | Event::Unknown(_)) => {
            // Spurious event from concurrent test — acceptable, but NOT the
            // prior JobError. The test still proves broadcast semantics hold.
        }
    }

    let _ = shutdown_tx.send(true);
    let _ = server.await;
}
