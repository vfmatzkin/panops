import Foundation
import Testing
@testable import Panops

/// Records the capture params it was asked to start with, so tests can assert
/// `LiveRecordingController` forwards the chosen `RecordingOptions` to the wire.
private actor CapturingRecordingIpcClient: LiveRecordingIpcClient {
    private(set) var audioSources: AudioSourcesWire?
    private(set) var intervalMs: UInt64?
    private(set) var threshold: Float?

    func recordingStart(
        meetingId: String,
        audioSources: AudioSourcesWire,
        screenshotIntervalMs: UInt64,
        screenshotThreshold: Float
    ) async throws -> RecordingAccepted {
        self.audioSources = audioSources
        self.intervalMs = screenshotIntervalMs
        self.threshold = screenshotThreshold
        return RecordingAccepted(recordingId: "rec-1")
    }

    func recordingStop(recordingId: String) async throws -> RecordingStopped {
        RecordingStopped(systemAudioPath: nil, micAudioPath: nil, screenshotPaths: [], durationMs: 0)
    }
}

@Suite("RecordingController option passthrough")
@MainActor
struct RecordingControllerOptionsTests {
    @Test("chosen options are forwarded to recording.start")
    func forwardsChosenOptions() async throws {
        let fake = CapturingRecordingIpcClient()
        let controller = LiveRecordingController(ipcClient: fake)

        try await controller.start(
            meetingId: "m1",
            options: RecordingOptions(
                audioSources: .micOnly,
                screenshotIntervalMs: 1000,
                screenshotThreshold: 0.5
            )
        )

        #expect(await fake.audioSources == .micOnly)
        #expect(await fake.intervalMs == 1000)
        #expect(await fake.threshold == 0.5)
        #expect(controller.isRecording)
    }

    @Test("the no-options start uses engine-default capture options")
    func defaultStartUsesDefaults() async throws {
        let fake = CapturingRecordingIpcClient()
        let controller = LiveRecordingController(ipcClient: fake)

        try await controller.start(meetingId: "m1")

        #expect(await fake.audioSources == .systemAndMic)
        #expect(await fake.intervalMs == 500)
        #expect(await fake.threshold == 0.15)
    }
}
