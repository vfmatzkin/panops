import SwiftUI

/// Record/Stop control bar backed by an injected RecordingController.
struct RecordBar<Controller: RecordingController & ObservableObject>: View {
    @ObservedObject var controller: Controller
    let meetingId: String
    let onRecordingStopped: (URL?) async throws -> Void
    @State private var isBusy = false
    @State private var errorMessage: String?
    @State private var lastAudioURL: URL?

    init(
        controller: Controller,
        meetingId: String,
        onRecordingStopped: @escaping (URL?) async throws -> Void = { _ in }
    ) {
        self._controller = ObservedObject(wrappedValue: controller)
        self.meetingId = meetingId
        self.onRecordingStopped = onRecordingStopped
    }

    var body: some View {
        HStack(spacing: 16) {
            Button(action: {
                Task { @MainActor in
                    await toggleRecording()
                }
            }) {
                HStack(spacing: 4) {
                    Image(systemName: controller.isRecording ? "stop.circle" : "record.circle")
                        .foregroundStyle(controller.isRecording ? Color.primary : Color.red)
                    Text(controller.isRecording ? "Stop" : "Record")
                }
            }
            .disabled(isBusy)
            .help(controller.isRecording ? "Stop recording" : "Start recording")

            Spacer()

            Text(statusText)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding()
        .background(Color.secondary.opacity(0.05))
        .alert("Recording error", isPresented: errorPresented) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(errorMessage ?? "")
        }
    }

    private var statusText: String {
        if isBusy {
            return controller.isRecording ? "Stopping recording…" : "Starting recording…"
        }
        if controller.isRecording {
            return "Recording audio and screenshots"
        }
        if let lastAudioURL {
            return "Last audio: \(lastAudioURL.lastPathComponent)"
        }
        return "Ready to record"
    }

    private var errorPresented: Binding<Bool> {
        Binding(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )
    }

    private func toggleRecording() async {
        guard !isBusy else { return }
        isBusy = true
        let wasRecording = controller.isRecording
        defer { isBusy = false }

        do {
            if wasRecording {
                let audioURL = try await controller.stop()
                if let audioURL {
                    guard PathValidator.isUnderPanopsDataDir(audioURL.path) else {
                        throw RecordingPathValidationError.unsafePath(audioURL.path)
                    }
                    lastAudioURL = audioURL
                } else {
                    lastAudioURL = nil
                }
                try await onRecordingStopped(audioURL)
            } else {
                lastAudioURL = nil
                try await controller.start(meetingId: meetingId)
            }
        } catch {
            AppViewModel.logFullError(wasRecording ? "recording.stop" : "recording.start", error)
            errorMessage = wasRecording ? "Couldn't stop recording." : "Couldn't start recording."
        }
    }
}
