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

/// Health-line poll cadence (~1 Hz): how often the recording screen re-reads the
/// meeting directory's on-disk size, the override for the dropped sidecar
/// health-event channel. File-scope because `RecordingScreen` is generic and
/// can't hold a static stored property.
private let healthPollIntervalNanoseconds: UInt64 = 1_000_000_000

/// The full recording screen: the app's own live preview (the same source the
/// sidecar records), live mic/system meters, a recording-health line proving
/// bytes are landing on disk, and a prominent Stop. Shown while
/// `controller.isRecording`. Deliberately shows no live transcript — transcript
/// + notes are produced only after Stop.
struct RecordingScreen<Controller: RecordingController & ObservableObject>: View {
    @ObservedObject var controller: Controller
    @ObservedObject var preview: CapturePreviewController
    let setup: RecordingSetup
    /// Meeting directory whose growing artifacts the health line polls.
    let meetingDirPath: String?
    let onRecordingStopped: (RecordingStopOutcome) async throws -> Void

    @State private var isStopping = false
    @State private var errorMessage: String?
    @State private var startDate: Date?
    @State private var bytesWritten: Int64 = 0

    init(
        controller: Controller,
        preview: CapturePreviewController,
        setup: RecordingSetup,
        meetingDirPath: String?,
        onRecordingStopped: @escaping (RecordingStopOutcome) async throws -> Void = { _ in }
    ) {
        self._controller = ObservedObject(wrappedValue: controller)
        self._preview = ObservedObject(wrappedValue: preview)
        self.setup = setup
        self.meetingDirPath = meetingDirPath
        self.onRecordingStopped = onRecordingStopped
    }

    var body: some View {
        VStack(spacing: 18) {
            header
            previewPane
            metersRow
            healthLine
            stopButton
            TrustStrip()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
        .onAppear { if startDate == nil { startDate = Date() } }
        .task(id: meetingDirPath) { await pollHealth() }
        .alert("Recording error", isPresented: errorPresented) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(errorMessage ?? "")
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Circle().fill(Color.red).frame(width: 11, height: 11)
            Text("Recording").font(.headline).foregroundStyle(.secondary)
            TimelineView(.periodic(from: startDate ?? Date(), by: 1)) { context in
                Text(RecordingClock.label(seconds: elapsedSeconds(asOf: context.date)))
                    .font(.system(size: 28, weight: .semibold, design: .rounded))
                    .monospacedDigit()
            }
        }
    }

    /// The live preview, when the app's preview stream is running. Falls back to
    /// a neutral note while it spins up or if it couldn't start.
    @ViewBuilder
    private var previewPane: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 10).fill(Color.black.opacity(0.9))
            if preview.state == .live {
                CapturePreviewView(layer: preview.displayLayer)
            } else {
                Text("Recording in progress — preview unavailable.")
                    .font(.callout)
                    .foregroundStyle(.white.opacity(0.8))
            }
        }
        .frame(maxWidth: 560, maxHeight: 300)
        .clipShape(RoundedRectangle(cornerRadius: 10))
    }

    private var metersRow: some View {
        VStack(spacing: 6) {
            if setup.audioSources != .micOnly {
                LevelMeter(label: "System", systemImage: "speaker.wave.2", db: preview.systemDb)
            }
            if setup.audioSources != .systemOnly {
                LevelMeter(label: "Mic", systemImage: "mic", db: preview.micDb)
            }
        }
        .frame(maxWidth: 360)
    }

    private var healthLine: some View {
        TimelineView(.periodic(from: startDate ?? Date(), by: 1)) { context in
            Text(healthText(asOf: context.date))
                .font(.callout.monospacedDigit())
                .foregroundStyle(bytesWritten > 0 ? .secondary : Color.orange)
        }
    }

    private var stopButton: some View {
        Button(role: .destructive) {
            Task { @MainActor in await stop() }
        } label: {
            Label("Stop Recording", systemImage: "stop.fill").padding(.horizontal, 12)
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
        .tint(.red)
        // Disabled until the engine accepts the start (canStop). `isRecording`
        // flips optimistically before acceptance, so gating on it alone would
        // show an enabled Stop that no-ops during a slow start.
        .disabled(isStopping || !controller.canStop)
        .help("Stop recording and generate notes")
    }

    /// "● MM:SS · 1.2 MB" — the elapsed clock plus the on-disk size, the proof
    /// that audio/video are actually being written. Orange until bytes appear.
    private func healthText(asOf now: Date) -> String {
        let clock = RecordingClock.label(seconds: elapsedSeconds(asOf: now))
        if bytesWritten > 0 {
            return "● \(clock) · \(Self.humanBytes(bytesWritten)) written"
        }
        return "● \(clock) · waiting for data…"
    }

    /// Poll the meeting directory's growing artifacts (~1 Hz) so the health line
    /// reflects real bytes on disk — the override for the dropped sidecar
    /// health-event channel.
    private func pollHealth() async {
        while !Task.isCancelled {
            bytesWritten = Self.recordingBytes(in: meetingDirPath)
            try? await Task.sleep(nanoseconds: healthPollIntervalNanoseconds)
        }
    }

    private static func recordingBytes(in dirPath: String?) -> Int64 {
        guard let dirPath, PathValidator.isUnderPanopsDataDir(dirPath) else { return 0 }
        var total: Int64 = 0
        for name in ["recording.mov", "system.wav", "mic.wav"] {
            let path = (dirPath as NSString).appendingPathComponent(name)
            if let size = try? FileManager.default.attributesOfItem(atPath: path)[.size] as? Int64 {
                total += size
            }
        }
        return total
    }

    private static func humanBytes(_ bytes: Int64) -> String {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        return formatter.string(fromByteCount: bytes)
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
