import SwiftUI

/// Modal setup sheet shown before a recording starts. Collects the title,
/// language, audio sources, and screenshot preference, then hands a
/// `RecordingSetup` back via `onStart` — the caller drives the existing
/// `meeting.start` → `recording.start` flow with it.
struct NewRecordingSheet: View {
    /// Display order for the audio picker: the most common pick first.
    private static let audioChoices: [AudioSourcesWire] = [.systemAndMic, .micOnly, .systemOnly]

    let onStart: (RecordingSetup) -> Void
    let onCancel: () -> Void

    @State private var setup = RecordingSetup.default

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("New Recording")
                .font(.title2.weight(.semibold))
                .padding(.horizontal, 20)
                .padding(.top, 20)
                .padding(.bottom, 12)

            Form {
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
                    Text("Record + notes")
                        .tag(true)
                    Text("Record only")
                        .tag(false)
                }
                .pickerStyle(.segmented)

                Text("Capture now, generate notes later — good when Ollama / Apple Intelligence is unavailable.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .formStyle(.grouped)

            HStack {
                TrustStrip()
                Spacer()
                Button("Cancel", role: .cancel) { onCancel() }
                    .keyboardShortcut(.cancelAction)
                Button("Start") { onStart(setup) }
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
            }
            .padding(20)
        }
        .frame(width: 420)
    }
}
