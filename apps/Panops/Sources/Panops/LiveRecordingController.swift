import Foundation
import Combine

protocol LiveRecordingIpcClient: Sendable {
    func recordingStart(
        meetingId: String,
        audioSources: AudioSourcesWire,
        screenshotIntervalMs: UInt64,
        screenshotThreshold: Float
    ) async throws -> RecordingAccepted

    func recordingStop(recordingId: String) async throws -> RecordingStopped
}

extension IpcClient: LiveRecordingIpcClient {}

enum RecordingPathValidationError: Error {
    case unsafePath(String)
}

/// Live recording controller backed by the panops engine IPC.
@MainActor
final class LiveRecordingController: RecordingController, ObservableObject {
    @Published private(set) var isRecording = false

    private let ipcClient: any LiveRecordingIpcClient
    private var recordingId: String?

    init(ipcClient: any LiveRecordingIpcClient) {
        self.ipcClient = ipcClient
    }

    func start(meetingId: String) async throws {
        guard !isRecording else { return }
        isRecording = true
        recordingId = nil

        do {
            let accepted = try await ipcClient.recordingStart(
                meetingId: meetingId,
                audioSources: .systemAndMic,
                screenshotIntervalMs: 500,
                screenshotThreshold: 0.15
            )
            recordingId = accepted.recordingId
        } catch {
            isRecording = false
            recordingId = nil
            throw error
        }
    }

    func stop() async throws -> URL? {
        guard let activeRecordingId = recordingId else { return nil }

        // Clear state only AFTER the engine confirms the stop. If
        // `recordingStop` throws (transient/engine error), keep recordingId +
        // isRecording so the user can retry Stop instead of orphaning the
        // engine-side recording (the error is surfaced by the caller). On
        // success we clear even if path validation then fails — the recording
        // did stop.
        let stopped = try await ipcClient.recordingStop(recordingId: activeRecordingId)
        recordingId = nil
        isRecording = false
        try validateArtifactPaths(stopped)

        let audioPath = stopped.systemAudioPath ?? stopped.micAudioPath
        return audioPath.map { URL(fileURLWithPath: $0) }
    }

    private func validateArtifactPaths(_ stopped: RecordingStopped) throws {
        let paths = [stopped.systemAudioPath, stopped.micAudioPath].compactMap { $0 }
            + stopped.screenshotPaths
        for path in paths {
            guard PathValidator.isUnderPanopsDataDir(path) else {
                throw RecordingPathValidationError.unsafePath(path)
            }
        }
    }
}
