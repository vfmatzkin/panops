//! Shared broadcast channel for the `events.subscribe` subscription.
//!
//! All `events.subscribe` subscribers share one `tokio::sync::broadcast`
//! sender. Wave 5K's `notes.generate` handler posts `Event::JobDone` /
//! `Event::JobError` here; per-connection subscribers fan-out via
//! `Sender::subscribe()`. Late subscribers miss earlier events
//! (broadcast semantics) — replay deferred (slice 05 spec §D6).

use panops_protocol::Event;
use tokio::sync::broadcast;

/// Capacity tuned for slice 05's single-user / few-jobs workload. A
/// laggy client gets `RecvError::Lagged` instead of stalling the
/// producer, which the subscription handler swallows.
const EVENT_CHANNEL_CAPACITY: usize = 256;

pub(super) fn channel() -> (broadcast::Sender<Event>, broadcast::Receiver<Event>) {
    broadcast::channel(EVENT_CHANNEL_CAPACITY)
}
