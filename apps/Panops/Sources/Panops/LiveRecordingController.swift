import Foundation
import Combine

/// Live recording controller backed by the panops engine IPC.
@MainActor
final class LiveRecordingController: RecordingController, ObservableObject {
    @Published private(set) var isRecording = false

    private let ipcClient: IpcClient
    private var recordingId: String?

    init(ipcClient: IpcClient) {
        self.ipcClient = ipcClient
    }

    func start(meetingId: String) async throws {
        guard !isRecording else { return }

        let accepted = try await ipcClient.recordingStart(
            meetingId: meetingId,
            audioSources: .systemAndMic,
            screenshotIntervalMs: 500,
            screenshotThreshold: 0.15
        )
        recordingId = accepted.recordingId
        isRecording = true
    }

    func stop() async throws -> URL? {
        guard let recordingId else { return nil }

        let stopped = try await ipcClient.recordingStop(recordingId: recordingId)
        self.recordingId = nil
        isRecording = false

        let audioPath = stopped.systemAudioPath ?? stopped.micAudioPath
        return audioPath.map { URL(fileURLWithPath: $0) }
    }
}
