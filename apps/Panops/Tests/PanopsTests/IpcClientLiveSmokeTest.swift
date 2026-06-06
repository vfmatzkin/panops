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
/// Tests exercise meeting lifecycle + notes generation with fixture audio,
/// asserting actual event delivery via WebSocket subscription.
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

        // Use fixture audio for deterministic test
        // Expect PANOPS_FIXTURES_DIR env var or use default path relative to repo
        let fixturesDir = ProcessInfo.processInfo.environment["PANOPS_FIXTURES_DIR"]
            ?? FileManager.default.currentDirectoryPath + "/tests/fixtures"
        let audioPath = URL(fileURLWithPath: fixturesDir).appendingPathComponent("audio/en_30s.wav")

        // Verify fixture exists
        guard FileManager.default.fileExists(atPath: audioPath.path) else {
            Issue.record("fixture audio not found at \(audioPath.path). Set PANOPS_FIXTURES_DIR or run from repo root.")
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
        let meetingId = try await client.meetingStart()
        let jobId = try await client.notesGenerate(audio: audioPath, meetingId: meetingId)
        #expect(!jobId.isEmpty, "notes.generate should return a job_id")

        // Wait for job.done or job.error with a short timeout (30s for short audio)
        var receivedEvent: IpcEvent?
        let timeout = Date().addingTimeInterval(30)

        for await event in eventStream {
            switch event {
            case .jobDone(let jId, _):
                if jId == jobId {
                    receivedEvent = event
                    break
                }
            case .jobError(let jId, _):
                if jId == jobId {
                    receivedEvent = event
                    break
                }
            case .unknown:
                break
            }
            if receivedEvent != nil || Date() > timeout {
                break
            }
        }

        // Assert we received a job completion event for our job_id
        #expect(receivedEvent != nil, "expected job.done or job.error for job_id \(jobId) within 30s")
        if let event = receivedEvent {
            switch event {
            case .jobDone(let jId, let result):
                #expect(jId == jobId, "job.done job_id mismatch")
                #expect(result.meetingId == meetingId, "job.done meeting_id mismatch")
            case .jobError(let jId, let payload):
                #expect(jId == jobId, "job.error job_id mismatch")
                // job.error is acceptable (e.g., ASR model unavailable)
                // Log but don't fail - we're testing event delivery, not pipeline success
                print("job.error received: \(payload.kind) - \(payload.message)")
            case .unknown:
                Issue.record("unexpected unknown event")
            }
        }

        await client.disconnect()
    }
}
