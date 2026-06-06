import Foundation

/// Central dispatcher for IPC events from WebSocket subscription.
/// Routes `job.done`/`job.error` to registered callbacks keyed by `job_id`.
/// Buffers recent events to prevent lost-wakeup race: if a job.done/job.error
/// arrives before registerCallback is called, the event is replayed on register.
/// Slice 12 implementation per spec D7.
actor EventStreamActor {
    private var subscription: AsyncStream<IpcEvent>?
    private var callbacks: [String: @Sendable (IpcEvent) -> Void] = [:]
    /// Buffer of recent job.done/job.error events, stored in insertion order.
    /// Bounded to prevent unbounded growth if callbacks are never registered.
    private var eventBuffer: [(jobId: String, event: IpcEvent)] = []
    private static let maxBufferSize = 64

    /// Subscribe to events from the IPC client.
    /// Stores the subscription and starts routing events to callbacks.
    func subscribe(client: IpcClient) async throws {
        let stream = try await client.subscribeEvents()
        subscription = stream
        Task {
            for await event in stream {
                route(event: event)
            }
        }
    }

    /// Register a callback for a specific job_id.
    /// The callback is invoked when `job.done` or `job.error` arrives for that job.
    /// If the event was already received (lost-wakeup race), replay immediately.
    func registerCallback(jobId: String, handler: @escaping @Sendable (IpcEvent) -> Void) {
        // Replay buffered event if already received (lost-wakeup race fix)
        if let idx = eventBuffer.firstIndex(where: { $0.jobId == jobId }) {
            handler(eventBuffer[idx].event)
            eventBuffer.remove(at: idx)
            return
        }
        callbacks[jobId] = handler
    }

    /// Unregister a callback for a specific job_id.
    /// Called after the job completes to clean up.
    func unregisterCallback(jobId: String) {
        callbacks.removeValue(forKey: jobId)
        eventBuffer.removeAll(where: { $0.jobId == jobId })
    }

    /// Stop the subscription and clear all callbacks.
    func stop() {
        subscription = nil
        callbacks.removeAll()
        eventBuffer.removeAll()
    }

    /// Route an event to its registered callback (if any).
    /// If no callback is registered yet, buffer the event for replay.
    private func route(event: IpcEvent) {
        switch event {
        case .jobDone(let jobId, _), .jobError(let jobId, _):
            if let handler = callbacks[jobId] {
                handler(event)
                // Auto-unregister after delivery
                callbacks.removeValue(forKey: jobId)
            } else {
                // No callback registered yet — buffer for replay
                if eventBuffer.count >= Self.maxBufferSize {
                    // Evict oldest entry (first in insertion order)
                    eventBuffer.removeFirst()
                }
                eventBuffer.append((jobId: jobId, event: event))
            }
        case .unknown:
            // Ignore unknown events
            break
        }
    }

    #if DEBUG
    /// Test-only helper to directly route an event.
    func testRoute(event: IpcEvent) {
        route(event: event)
    }
    #endif
}
