import SwiftUI

/// Modal setup sheet shown before a recording starts. Collects the title,
/// language, audio sources, and screenshot preference, then hands a
/// `RecordingSetup` back via `onStart` — the caller drives the existing
/// `meeting.start` → `recording.start` flow with it. The capture source +
/// resolution + live preview are added by the system-picker flow.
struct NewRecordingSheet: View {
    /// Display order for the audio picker: the most common pick first.
    private static let audioChoices: [AudioSourcesWire] = [.systemAndMic, .micOnly, .systemOnly]

    let onStart: (RecordingSetup) -> Void
    let onCancel: () -> Void

    @State private var setup = RecordingSetup.default
    @StateObject private var preview = CapturePreviewController()

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("New Recording")
                .font(.title2.weight(.semibold))
                .padding(.horizontal, 20)
                .padding(.top, 20)
                .padding(.bottom, 12)

            settingsForm

            footer
        }
        .frame(width: 460)
        .onDisappear { preview.teardown() }
    }

    private var settingsForm: some View {
        Form {
            Section("Capture") {
                CaptureSourcePane(controller: preview, resolution: $setup.resolution)
            }

            TextField("Title", text: $setup.title, prompt: Text("Optional"))

            Picker("Language", selection: $setup.language) {
                ForEach(RecordingLanguage.allCases) { language in
                    Text(language.label).tag(language)
                }
            }

            Picker("Audio", selection: $setup.audioSources) {
                ForEach(Self.audioChoices, id: \.self) { source in
                    Text(source.displayLabel).tag(source)
                }
            }

            VStack(alignment: .leading, spacing: 2) {
                Toggle("Capture screenshots", isOn: $setup.captureScreenshots)
                    .disabled(true)
                Text("Screenshots are always captured in this version.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Toggle("Record video", isOn: $setup.recordVideo)

            Picker("Processing mode", selection: $setup.autoGenerateNotes) {
                Text("Record + notes").tag(true)
                Text("Record only").tag(false)
            }
            .pickerStyle(.segmented)

            Text("Capture now, generate notes later — good when Ollama / Apple Intelligence is unavailable.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
    }

    private var footer: some View {
        HStack {
            TrustStrip()
            Spacer()
            Button("Cancel", role: .cancel) { onCancel() }
                .keyboardShortcut(.cancelAction)
            Button("Start") { start() }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
        }
        .padding(20)
    }

    /// Fold the live picker selection into the setup before handing it back: the
    /// chosen target (default primary display when nothing was picked) and the
    /// source's native height, which resolves the resolution preset on the wire.
    private func start() {
        var resolved = setup
        resolved.captureTarget = preview.target ?? .primaryDisplay
        resolved.captureNativeHeight = preview.nativePixelHeight
        onStart(resolved)
    }
}
