import Foundation
import Testing
@testable import Panops

@Suite("LiveRecordingController")
@MainActor
struct LiveRecordingControllerTests {
    @Test("optimistic start prevents a double-start while IPC is in flight")
    func optimisticStartPreventsDoubleStart() async throws {
        let fake = FakeLiveRecordingIpcClient(startDelayNanoseconds: 50_000_000)
        let controller = LiveRecordingController(ipcClient: fake)

        let firstStart = Task { @MainActor in
            try await controller.start(meetingId: "meeting-1")
        }
        try await Task.sleep(nanoseconds: 1_000_000)

        #expect(controller.isRecording)
        try await controller.start(meetingId: "meeting-1")
        try await firstStart.value

        let startCalls = await fake.startCallCount()
        #expect(startCalls == 1)
        #expect(controller.isRecording)
    }

    @Test("start failure resets recording state")
    func startFailureResetsRecordingState() async throws {
        let fake = FakeLiveRecordingIpcClient(startError: TestRecordingError.startFailed)
        let controller = LiveRecordingController(ipcClient: fake)

        do {
            try await controller.start(meetingId: "meeting-1")
            Issue.record("start should throw")
        } catch {
            #expect(controller.isRecording == false)
            let startCalls = await fake.startCallCount()
            #expect(startCalls == 1)
        }
    }

    @Test("stop failure resets recording state")
    func stopFailureResetsRecordingState() async throws {
        let fake = FakeLiveRecordingIpcClient(stopError: TestRecordingError.stopFailed)
        let controller = LiveRecordingController(ipcClient: fake)

        try await controller.start(meetingId: "meeting-1")
        #expect(controller.isRecording)

        do {
            _ = try await controller.stop()
            Issue.record("stop should throw")
        } catch {
            #expect(controller.isRecording == false)
            let stopCalls = await fake.stopCallCount()
            #expect(stopCalls == 1)
        }

        let secondStop = try await controller.stop()
        let stopCalls = await fake.stopCallCount()
        #expect(secondStop == nil)
        #expect(stopCalls == 1)
    }
}

private enum TestRecordingError: Error {
    case startFailed
    case stopFailed
}

private actor FakeLiveRecordingIpcClient: LiveRecordingIpcClient {
    private var startCalls = 0
    private var stopCalls = 0
    private let startDelayNanoseconds: UInt64
    private let startError: (any Error)?
    private let stopError: (any Error)?
    private let accepted: RecordingAccepted
    private let stopped: RecordingStopped

    init(
        startDelayNanoseconds: UInt64 = 0,
        startError: (any Error)? = nil,
        stopError: (any Error)? = nil,
        accepted: RecordingAccepted = RecordingAccepted(recordingId: "recording-1"),
        stopped: RecordingStopped = RecordingStopped(
            systemAudioPath: (PathValidator.panopsDataRoot as NSString)
                .appendingPathComponent("meetings/meeting-1/system.wav"),
            micAudioPath: nil,
            screenshotPaths: [],
            durationMs: 1_000
        )
    ) {
        self.startDelayNanoseconds = startDelayNanoseconds
        self.startError = startError
        self.stopError = stopError
        self.accepted = accepted
        self.stopped = stopped
    }

    func recordingStart(
        meetingId: String,
        audioSources: AudioSourcesWire,
        screenshotIntervalMs: UInt64,
        screenshotThreshold: Float
    ) async throws -> RecordingAccepted {
        startCalls += 1
        if startDelayNanoseconds > 0 {
            try await Task.sleep(nanoseconds: startDelayNanoseconds)
        }
        if let startError {
            throw startError
        }
        return accepted
    }

    func recordingStop(recordingId: String) async throws -> RecordingStopped {
        stopCalls += 1
        if let stopError {
            throw stopError
        }
        return stopped
    }

    func startCallCount() -> Int {
        startCalls
    }

    func stopCallCount() -> Int {
        stopCalls
    }
}
