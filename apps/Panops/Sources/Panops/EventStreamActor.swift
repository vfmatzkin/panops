import Foundation

/// Central dispatcher for IPC events from WebSocket subscription.
/// Routes `job.done`/`job.error` to registered callbacks keyed by `job_id`.
/// Slice 12 implementation per spec D7.
actor EventStreamActor {
    private var subscription: AsyncStream<IpcEvent>?
    private var callbacks: [String: @Sendable (IpcEvent) -> Void] = [:]
    private var subscriptionTask: Task<Void, Never>?

    /// Subscribe to events from the IPC client.
    /// Stores the subscription and starts routing events to callbacks.
    func subscribe(client: IpcClient) async throws {
        subscription = try await client.subscribeEvents()
        subscriptionTask = Task {
            for await event in subscription! {
                route(event: event)
            }
        }
    }

    /// Register a callback for a specific job_id.
    /// The callback is invoked when `job.done` or `job.error` arrives for that job.
    func registerCallback(jobId: String, handler: @escaping @Sendable (IpcEvent) -> Void) {
        callbacks[jobId] = handler
    }

    /// Unregister a callback for a specific job_id.
    /// Called after the job completes to clean up.
    func unregisterCallback(jobId: String) {
        callbacks.removeValue(forKey: jobId)
    }

    /// Stop the subscription and clear all callbacks.
    func stop() {
        subscriptionTask?.cancel()
        subscriptionTask = nil
        subscription = nil
        callbacks.removeAll()
    }

    /// Route an event to its registered callback (if any).
    private func route(event: IpcEvent) {
        switch event {
        case .jobDone(let jobId, _), .jobError(let jobId, _):
            if let handler = callbacks[jobId] {
                handler(event)
                // Auto-unregister after delivery
                callbacks.removeValue(forKey: jobId)
            }
        case .unknown:
            // Ignore unknown events
            break
        }
    }

    /// Test-only helper to directly route an event.
    func testRoute(event: IpcEvent) {
        route(event: event)
    }
}