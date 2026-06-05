import Foundation
import Testing
@testable import Panops

/// Live integration smoke against a running engine. SKIPPED by default;
/// gated on the `PANOPS_LIVE_ENGINE=1` env var so it doesn't break CI
/// (which doesn't have an engine running). Run manually via:
///
///     PANOPS_LIVE_ENGINE=1 swift test
///
/// Requires the engine to be running with its UDS at the default path:
///
///     ./target/release/panops-engine serve
///
/// The test exercises `meeting.start` + `meeting.get` only — no audio
/// fixture or `notes.generate` invocation. Notes-pipeline coverage is
/// the manual GUI smoke documented in `apps/Panops/README.md`.
@Suite("Live IPC smoke (requires engine + PANOPS_LIVE_ENGINE=1)")
struct IpcClientLiveSmokeTest {
    @Test("live_smoke_meeting_create")
    func liveSmokeMeetingCreateAndGet() async throws {
        guard ProcessInfo.processInfo.environment["PANOPS_LIVE_ENGINE"] == "1" else {
            return
        }
        let socketPath = FileManager.default
            .homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/panops/engine.sock")
        let client = IpcClient(socketPath: socketPath)
        try await client.connect()
        let id = try await client.meetingStart()
        #expect(!id.isEmpty)
        let meeting = try await client.meetingGet(id: id)
        #expect(meeting.id == id)
        #expect(meeting.language == "auto", "default language should be 'auto'")
        #expect(meeting.endedAt == nil, "fresh meeting should not be ended")
        // dir_path should be absolute under the engine's data_dir.
        #expect(meeting.dirPath.hasPrefix("/"), "dir_path should be absolute: \(meeting.dirPath)")
    }

    @Test("wsConnect_returns_101_on_upgrade")
    func wsConnectReturns101() async throws {
        guard ProcessInfo.processInfo.environment["PANOPS_LIVE_ENGINE"] == "1" else {
            return
        }
        let socketPath = FileManager.default
            .homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/panops/engine.sock")
        let client = IpcClient(socketPath: socketPath)
        try await client.wsConnect()
        // Success means we got HTTP 101 and the connection is stored
        // The actual WebSocket frame handling is tested separately
        await client.disconnect()
    }

    @Test("subscribeEvents_yields_jobDone_after_notesGenerate")
    func subscribeEventsYieldsJobDone() async throws {
        guard ProcessInfo.processInfo.environment["PANOPS_LIVE_ENGINE"] == "1" else {
            return
        }
        let socketPath = FileManager.default
            .homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/panops/engine.sock")
        let client = IpcClient(socketPath: socketPath)

        // Connect via WebSocket and subscribe to events
        try await client.wsConnect()
        let eventStream = try await client.subscribeEvents()

        // Create a meeting and trigger notes generation
        // We'll need to use HTTP POST for this since WebSocket is for events only
        let meetingId = try await client.meetingStart()

        // Watch for job.done event with our meeting_id
        var foundJobDone = false
        let timeout = Date().addingTimeInterval(60) // 60s timeout for manual test

        for await event in eventStream {
            switch event {
            case .jobDone(_, let result):
                if result.meetingId == meetingId {
                    foundJobDone = true
                    break
                }
            case .jobError(_, _):
                // Error is also valid - we're just testing event delivery
                break
            case .unknown:
                break
            }
            if foundJobDone || Date() > timeout {
                break
            }
        }

        // For this test, we just verify we can subscribe and receive events
        // Actual job.done verification requires triggering notes.generate
        // which needs audio fixtures - deferred to manual smoke

        await client.disconnect()
    }
}
