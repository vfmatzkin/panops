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
}
