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

use std::sync::Arc;

use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::ErrorObjectOwned;
use panops_protocol::{Event, IpcError, JobAccepted, MeetingSummary, NotesGenerateParams};
use tokio::sync::broadcast;

#[rpc(server, namespace = "ipc", namespace_separator = ".")]
pub trait Ipc {
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

pub struct IpcImpl {
    /// Reserved for Wave 5K — the `notes.generate` handler will reach
    /// into `services` for ASR / diar / LLM / exporter. The slice 05
    /// stubs don't need it yet but keeping the field here means Wave 5K
    /// is a one-handler edit instead of a struct re-shape.
    #[allow(dead_code)]
    pub services: Arc<crate::server::EngineServices>,
    pub events_tx: broadcast::Sender<Event>,
}

#[async_trait::async_trait]
impl IpcServer for IpcImpl {
    async fn notes_generate(
        &self,
        _params: NotesGenerateParams,
    ) -> Result<JobAccepted, ErrorObjectOwned> {
        // Wave 5K replaces this stub with the real pipeline call.
        Err(ipc_error_to_obj(IpcError::Internal {
            message: "notes.generate not yet wired (slice 05 Wave 5K)".into(),
        }))
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

/// Map `IpcError` to a JSON-RPC server error (-32000) carrying the
/// typed kind in `data` and the human-readable message at top level.
/// Mirrors the spec's "Error mapping at the RPC boundary" section.
pub fn ipc_error_to_obj(e: IpcError) -> ErrorObjectOwned {
    let data = serde_json::to_value(&e).expect("IpcError serialise");
    ErrorObjectOwned::owned(-32000, e.to_string(), Some(data))
}
