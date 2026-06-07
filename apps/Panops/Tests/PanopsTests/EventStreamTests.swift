import Foundation
import Testing
@testable import Panops

/// Thread-safe state wrapper for testing callbacks
final class TestState<T: Sendable>: @unchecked Sendable {
    var value: T?
    init() {}
}

@Suite("Event routing")
struct EventStreamTests {
    @Test("EventStreamActor_routes_jobDone_to_callback")
    func routesJobDoneToCallback() async throws {
        let actor = EventStreamActor()
        let state = TestState<IpcEvent>()

        await actor.registerCallback(jobId: "test-job-123", handler: { event in
            state.value = event
        })

        let event = IpcEvent.jobDone(
            jobId: "test-job-123",
            result: JobDoneResult(
                primaryFile: "/tmp/notes.md",
                assets: [],
                meetingId: "meeting-1"
            )
        )

        await actor.testRoute(event: event)

        #expect(state.value != nil, "callback should have been invoked")
        guard case .jobDone(let jobId, _) = state.value else {
            Issue.record("expected .jobDone, got \(String(describing: state.value))")
            return
        }
        #expect(jobId == "test-job-123")
    }

    @Test("EventStreamActor_routes_jobError_to_callback")
    func routesJobErrorToCallback() async throws {
        let actor = EventStreamActor()
        let state = TestState<IpcEvent>()

        await actor.registerCallback(jobId: "error-job-456", handler: { event in
            state.value = event
        })

        let event = IpcEvent.jobError(
            jobId: "error-job-456",
            error: JobErrorPayload(kind: "internal", message: "something went wrong")
        )

        await actor.testRoute(event: event)

        #expect(state.value != nil, "callback should have been invoked")
        guard case .jobError(let jobId, _) = state.value else {
            Issue.record("expected .jobError, got \(String(describing: state.value))")
            return
        }
        #expect(jobId == "error-job-456")
    }

    @Test("EventStreamActor_routes_jobProgress_then_terminal_done_to_callback")
    func routesJobProgressThenTerminalDoneToCallback() async throws {
        let actor = EventStreamActor()
        let state = TestState<[IpcEvent]>()
        state.value = []

        await actor.registerCallback(jobId: "progress-job-789", handler: { event in
            state.value?.append(event)
        })

        await actor.testRoute(event: .jobProgress(JobProgressEvent(
            jobId: "progress-job-789",
            stage: "transcribing",
            current: 1,
            total: 3,
            message: "mic track"
        )))
        await actor.testRoute(event: .jobDone(
            jobId: "progress-job-789",
            result: JobDoneResult(
                primaryFile: "/tmp/notes.md",
                assets: [],
                meetingId: "meeting-1"
            )
        ))

        #expect(state.value?.count == 2, "progress should not unregister terminal callback")
        guard case .jobProgress(let progress) = state.value?.first else {
            Issue.record("expected first event to be .jobProgress, got \(String(describing: state.value?.first))")
            return
        }
        #expect(progress.jobId == "progress-job-789")
        #expect(progress.stage == "transcribing")

        guard case .jobDone(let jobId, _) = state.value?.last else {
            Issue.record("expected last event to be .jobDone, got \(String(describing: state.value?.last))")
            return
        }
        #expect(jobId == "progress-job-789")
    }

    @Test("EventStreamActor_ignores_unknown_events")
    func ignoresUnknownEvents() async throws {
        let actor = EventStreamActor()
        let invoked = TestState<Bool>()
        invoked.value = false

        await actor.registerCallback(jobId: "any-job", handler: { _ in
            invoked.value = true
        })

        let event = IpcEvent.unknown(type: "asr.partial")
        await actor.testRoute(event: event)

        #expect(invoked.value == false, "unknown events should not trigger callbacks")
    }

    @Test("EventStreamActor_ignores_missing_job_id")
    func ignoresMissingJobId() async throws {
        let actor = EventStreamActor()
        let invoked = TestState<Bool>()
        invoked.value = false

        await actor.registerCallback(jobId: "registered-job", handler: { _ in
            invoked.value = true
        })

        // Send event for a job_id that wasn't registered
        let event = IpcEvent.jobDone(
            jobId: "unregistered-job",
            result: JobDoneResult(
                primaryFile: "/tmp/notes.md",
                assets: [],
                meetingId: "meeting-1"
            )
        )
        await actor.testRoute(event: event)

        #expect(invoked.value == false, "events for unregistered job_ids should be ignored")
    }

    @Test("EventStreamActor_auto_unregisters_after_delivery")
    func autoUnregistersAfterDelivery() async throws {
        let actor = EventStreamActor()
        let count = TestState<Int>()
        count.value = 0

        await actor.registerCallback(jobId: "auto-unregister-job", handler: { _ in
            count.value = (count.value ?? 0) + 1
        })

        let event = IpcEvent.jobDone(
            jobId: "auto-unregister-job",
            result: JobDoneResult(
                primaryFile: "/tmp/notes.md",
                assets: [],
                meetingId: "meeting-1"
            )
        )

        // Route the same event twice
        await actor.testRoute(event: event)
        await actor.testRoute(event: event)

        #expect(count.value == 1, "callback should only be called once after auto-unregister")
    }
}
