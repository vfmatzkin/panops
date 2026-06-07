import SwiftUI

/// Record/Stop control bar backed by an injected RecordingController.
struct RecordBar<Controller: RecordingController & ObservableObject>: View {
    @ObservedObject var controller: Controller
    let meetingId: String
    @State private var isBusy = false
    @State private var errorMessage: String?
    @State private var lastAudioURL: URL?

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
        defer { isBusy = false }

        do {
            if controller.isRecording {
                lastAudioURL = try await controller.stop()
            } else {
                lastAudioURL = nil
                try await controller.start(meetingId: meetingId)
            }
        } catch let IpcClientError.rpcError(_, message) {
            errorMessage = message
        } catch {
            AppViewModel.logFullError("recording.toggle", error)
            errorMessage = "Could not reach the engine."
        }
    }
}
