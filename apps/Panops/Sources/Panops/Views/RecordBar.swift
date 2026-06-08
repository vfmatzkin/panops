import SwiftUI

/// Elapsed-time formatting for the recording screen. Pure + testable; the
/// timer is client-side (counts up from the moment the screen appears) — the
/// engine emits no authoritative recording clock.
enum RecordingClock {
    /// "MM:SS" under an hour, "H:MM:SS" once it passes one. Negatives clamp to
    /// zero so a clock-skew blip never renders a negative timer.
    static func label(seconds: Int) -> String {
        let total = max(0, seconds)
        let hours = total / 3600
        let minutes = (total % 3600) / 60
        let secs = total % 60
        if hours > 0 {
            return String(format: "%d:%02d:%02d", hours, minutes, secs)
        }
        return String(format: "%02d:%02d", minutes, secs)
    }
}

/// The full recording screen: a large client-side timer, the capture-source
/// indicators chosen in the setup sheet, the honest trust strip, and a
/// prominent Stop. Shown while `controller.isRecording`. Deliberately shows no
/// live transcript — transcript + notes are produced only after Stop.
struct RecordingScreen<Controller: RecordingController & ObservableObject>: View {
    @ObservedObject var controller: Controller
    let setup: RecordingSetup
    let onRecordingStopped: (RecordingStopOutcome) async throws -> Void

    @State private var isStopping = false
    @State private var errorMessage: String?
    @State private var startDate: Date?

    init(
        controller: Controller,
        setup: RecordingSetup,
        onRecordingStopped: @escaping (RecordingStopOutcome) async throws -> Void = { _ in }
    ) {
        self._controller = ObservedObject(wrappedValue: controller)
        self.setup = setup
        self.onRecordingStopped = onRecordingStopped
    }

    var body: some View {
        VStack(spacing: 24) {
            Spacer()

            HStack(spacing: 8) {
                Circle()
                    .fill(Color.red)
                    .frame(width: 11, height: 11)
                Text("Recording")
                    .font(.headline)
                    .foregroundStyle(.secondary)
            }

            TimelineView(.periodic(from: startDate ?? Date(), by: 1)) { context in
                Text(RecordingClock.label(seconds: elapsedSeconds(asOf: context.date)))
                    .font(.system(size: 60, weight: .semibold, design: .rounded))
                    .monospacedDigit()
            }

            captureSourceIndicators

            Text("Transcript & notes appear after recording stops.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            Button(role: .destructive) {
                Task { @MainActor in await stop() }
            } label: {
                Label("Stop Recording", systemImage: "stop.fill")
                    .padding(.horizontal, 12)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .tint(.red)
            // Disabled until the engine accepts the start (canStop). `isRecording`
            // flips optimistically before acceptance, so gating on it alone would
            // show an enabled Stop that no-ops during a slow start.
            .disabled(isStopping || !controller.canStop)
            .help("Stop recording and generate notes")

            Spacer()
            TrustStrip()
                .padding(.bottom, 12)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
        .onAppear { if startDate == nil { startDate = Date() } }
        .alert("Recording error", isPresented: errorPresented) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(errorMessage ?? "")
        }
    }

    /// Active capture sources, derived from the chosen setup. Static-from-config
    /// is honest here: there are no live capture-status events to reflect.
    private var captureSourceIndicators: some View {
        HStack(spacing: 8) {
            ForEach(activeSources) { source in
                TrustChip(systemImage: source.icon, label: source.label)
            }
        }
    }

    private struct CaptureSource: Identifiable {
        let id: String
        let icon: String
        let label: String
    }

    private var activeSources: [CaptureSource] {
        // One audio chip labelled from the shared `displayLabel` so its wording
        // matches the New Recording picker exactly, plus the screenshots chip.
        var sources: [CaptureSource] = [
            CaptureSource(
                id: "audio",
                icon: setup.audioSources.icon,
                label: setup.audioSources.displayLabel
            )
        ]
        if setup.captureScreenshots {
            sources.append(CaptureSource(id: "screenshots", icon: "photo", label: "Screenshots"))
        }
        return sources
    }

    private func elapsedSeconds(asOf now: Date) -> Int {
        guard let startDate else { return 0 }
        return Int(now.timeIntervalSince(startDate))
    }

    private var errorPresented: Binding<Bool> {
        Binding(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )
    }

    private func stop() async {
        guard !isStopping else { return }
        isStopping = true
        defer { isStopping = false }
        do {
            let outcome = try await controller.stop()
            try await onRecordingStopped(outcome)
        } catch {
            AppViewModel.logFullError("recording.stop", error)
            errorMessage = "Couldn't stop recording."
        }
    }
}

/// Record/Stop control bar backed by an injected RecordingController.
struct RecordBar<Controller: RecordingController & ObservableObject>: View {
    @ObservedObject var controller: Controller
    let meetingId: String
    let onRecordingStarted: (String) async throws -> Void
    let onRecordingStopped: (RecordingStopOutcome) async throws -> Void
    @State private var isBusy = false
    @State private var errorMessage: String?
    @State private var lastAudioURL: URL?

    init(
        controller: Controller,
        meetingId: String,
        onRecordingStarted: @escaping (String) async throws -> Void = { _ in },
        onRecordingStopped: @escaping (RecordingStopOutcome) async throws -> Void = { _ in }
    ) {
        self._controller = ObservedObject(wrappedValue: controller)
        self.meetingId = meetingId
        self.onRecordingStarted = onRecordingStarted
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
                // The controller already validates returned artifact paths
                // (LiveRecordingController.validateArtifactPaths), so no
                // duplicate PathValidator check here.
                let outcome = try await controller.stop()
                lastAudioURL = outcome.audioURL
                try await onRecordingStopped(outcome)
            } else {
                lastAudioURL = nil
                try await controller.start(meetingId: meetingId)
                try await onRecordingStarted(meetingId)
            }
        } catch {
            AppViewModel.logFullError(wasRecording ? "recording.stop" : "recording.start", error)
            errorMessage = wasRecording ? "Couldn't stop recording." : "Couldn't start recording."
        }
    }
}
