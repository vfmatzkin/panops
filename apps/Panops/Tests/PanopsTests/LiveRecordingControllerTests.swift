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

    @Test("Stop is gated until recording.start is accepted")
    func canStopGatesUntilStartAccepted() async throws {
        let fake = FakeLiveRecordingIpcClient(startDelayNanoseconds: 50_000_000)
        let controller = LiveRecordingController(ipcClient: fake)

        #expect(controller.canStop == false)

        let firstStart = Task { @MainActor in
            try await controller.start(meetingId: "meeting-1")
        }
        try await Task.sleep(nanoseconds: 1_000_000)

        // Optimistic isRecording is up (blocks a double-start), but the engine
        // hasn't accepted yet — Stop must stay disabled while the start is in
        // flight, otherwise it no-ops and recording continues after acceptance.
        #expect(controller.isRecording)
        #expect(controller.canStop == false)

        try await firstStart.value
        // recording.start accepted (recordingId set) — Stop is now live.
        #expect(controller.canStop)

        _ = try await controller.stop()
        #expect(controller.canStop == false)
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

    @Test("stop failure keeps recording state for retry")
    func stopFailureKeepsRecordingStateForRetry() async throws {
        let fake = FakeLiveRecordingIpcClient(stopError: TestRecordingError.stopFailed)
        let controller = LiveRecordingController(ipcClient: fake)

        try await controller.start(meetingId: "meeting-1")
        #expect(controller.isRecording)

        do {
            _ = try await controller.stop()
            Issue.record("stop should throw")
        } catch {
            // State is kept on IPC failure so the user can retry Stop rather
            // than orphaning the engine-side recording.
            #expect(controller.isRecording == true)
            let stopCalls = await fake.stopCallCount()
            #expect(stopCalls == 1)
        }

        // Preserved state means a retry actually re-attempts the stop.
        do {
            _ = try await controller.stop()
            Issue.record("retry stop should throw again")
        } catch {
            let stopCalls = await fake.stopCallCount()
            #expect(stopCalls == 2)
        }
        #expect(controller.isRecording == true)
    }

    @Test("start records recordVideo and autoGenerateNotes flags")
    func startRecordsFlags() async throws {
        let fake = FakeLiveRecordingIpcClient()
        let controller = LiveRecordingController(ipcClient: fake)

        let options = RecordingOptions(recordVideo: true, autoGenerateNotes: false)
        try await controller.start(meetingId: "meeting-1", options: options)

        let (startCalls, lastRecordVideo, lastAutoGenerateNotes) = await fake.startCallCountAndRecordVideo()
        #expect(startCalls == 1)
        #expect(lastRecordVideo == true)
        #expect(lastAutoGenerateNotes == false)
    }

    @Test("start passes captureTarget to engine")
    func startRecordsCaptureTarget() async throws {
        let fake = FakeLiveRecordingIpcClient()
        let controller = LiveRecordingController(ipcClient: fake)

        let options = RecordingOptions(captureTarget: .window(windowId: 123))
        try await controller.start(meetingId: "meeting-1", options: options)

        let target = await fake.lastCaptureTargetValue()
        #expect(target == .window(windowId: 123))
    }

    @Test("stop surfaces the engine-issued auto-notes job id")
    func stopSurfacesNotesJobId() async throws {
        let stopped = RecordingStopped(
            systemAudioPath: (PathValidator.panopsDataRoot as NSString)
                .appendingPathComponent("meetings/meeting-1/system.wav"),
            micAudioPath: nil,
            screenshotPaths: [],
            durationMs: 1_000,
            notesJobId: "auto-job-1"
        )
        let fake = FakeLiveRecordingIpcClient(stopped: stopped)
        let controller = LiveRecordingController(ipcClient: fake)

        try await controller.start(
            meetingId: "meeting-1",
            options: RecordingOptions(autoGenerateNotes: true)
        )
        let outcome = try await controller.stop()

        #expect(outcome.notesJobId == "auto-job-1")
        #expect(outcome.autoGenerateNotesRequested == true)
        #expect(outcome.audioURL != nil)
    }

    @Test("stop flags auto-was-requested when the engine returns no job id")
    func stopFlagsDeferredWhenNoJobId() async throws {
        // The default fake `stopped` carries no notesJobId — the deferred case.
        let fake = FakeLiveRecordingIpcClient()
        let controller = LiveRecordingController(ipcClient: fake)

        try await controller.start(
            meetingId: "meeting-1",
            options: RecordingOptions(autoGenerateNotes: true)
        )
        let outcome = try await controller.stop()

        #expect(outcome.notesJobId == nil)
        #expect(outcome.autoGenerateNotesRequested == true)
    }

    @Test("stop reports auto-was-not-requested when the recording opted out")
    func stopReportsAutoNotRequested() async throws {
        let fake = FakeLiveRecordingIpcClient()
        let controller = LiveRecordingController(ipcClient: fake)

        try await controller.start(
            meetingId: "meeting-1",
            options: RecordingOptions(autoGenerateNotes: false)
        )
        let outcome = try await controller.stop()

        #expect(outcome.autoGenerateNotesRequested == false)
        #expect(outcome.notesJobId == nil)
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
    private var lastRecordVideo: Bool = false
    private var lastAutoGenerateNotes: Bool = false
    private var lastCaptureTarget: CaptureTarget = .display

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
        screenshotThreshold: Float,
        recordVideo: Bool,
        autoGenerateNotes: Bool,
        captureTarget: CaptureTarget
    ) async throws -> RecordingAccepted {
        startCalls += 1
        lastRecordVideo = recordVideo
        lastAutoGenerateNotes = autoGenerateNotes
        lastCaptureTarget = captureTarget
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

    func meetingDeleteVideo(meetingId: String) async throws -> (deleted: Bool, freedBytes: UInt64) {
        (deleted: true, freedBytes: 0)
    }

    func startCallCount() -> Int {
        startCalls
    }

    func stopCallCount() -> Int {
        stopCalls
    }

    func startCallCountAndRecordVideo() -> (Int, Bool, Bool) {
        (startCalls, lastRecordVideo, lastAutoGenerateNotes)
    }

    func captureWindows() async throws -> [WindowInfo] {
        []
    }

    func lastCaptureTargetValue() -> CaptureTarget {
        lastCaptureTarget
    }
}
