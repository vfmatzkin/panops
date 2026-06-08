import SwiftUI

/// Example windows for preview mode when IpcClient isn't available.
private struct IpcClientMock {
    static func captureWindows() async throws -> [WindowInfo] {
        return [
            WindowInfo(windowId: 1, appName: "Safari", title: "panops - Example Window"),
            WindowInfo(windowId: 2, appName: "Xcode", title: "PanopsApp.swift"),
            WindowInfo(windowId: 3, appName: "Terminal", title: "zsh")
        ]
    }
}

/// Modal setup sheet shown before a recording starts. Collects the title,
/// language, audio sources, capture target, and screenshot preference, then hands a
/// `RecordingSetup` back via `onStart` — the caller drives the existing
/// `meeting.start` → `recording.start` flow with it.
struct NewRecordingSheet: View {
    /// Display order for the audio picker: the most common pick first.
    private static let audioChoices: [AudioSourcesWire] = [.systemAndMic, .micOnly, .systemOnly]

    let onStart: (RecordingSetup) -> Void
    let onCancel: () -> Void

    @State private var setup = RecordingSetup.default
    @State private var windowList: [WindowInfo] = []
    @State private var isFetchingWindows = false
    @State private var windowsError: Error?
    @State private var selectedWindowId: UInt32?

    private enum CaptureTargetChoice: String, CaseIterable, Identifiable {
        case display = "Full display"
        case window = "Window…"

        var id: String { rawValue }
    }

    @State private var selectedChoice: CaptureTargetChoice = .display

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

                Picker("Capture target", selection: $selectedChoice) {
                    ForEach(CaptureTargetChoice.allCases) { choice in
                        Text(choice.rawValue).tag(choice)
                    }
                }
                .pickerStyle(.radioGroup)
                .onChange(of: selectedChoice) { _, newValue in
                    switch newValue {
                    case .display:
                        setup.captureTarget = .display
                        selectedWindowId = nil
                    case .window:
                        setup.captureTarget = .window(windowId: 0)  // Placeholder, will be set from list
                    }
                }

                if selectedChoice == .window {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            if isFetchingWindows {
                                ProgressView()
                            } else if let error = windowsError {
                                Text("Error: \(error.localizedDescription)")
                                    .foregroundColor(.red)
                                    .font(.caption)
                            } else {
                                Text("Select a window to capture")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            if !isFetchingWindows && windowList.isEmpty && windowsError == nil {
                                Button("Refresh") {
                                    Task { @MainActor in
                                        await fetchWindows()
                                    }
                                }
                                .font(.caption)
                            }
                        }

                        if isFetchingWindows {
                            Text("Loading windows…")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        } else if windowsError != nil {
                            Text("No windows available — recording the full display instead")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Button("Try again") {
                                Task { @MainActor in
                                    await fetchWindows()
                                }
                            }
                        } else if windowList.isEmpty {
                            Text("No windows available — recording the full display instead")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Button("Try again") {
                                Task { @MainActor in
                                    await fetchWindows()
                                }
                            }
                        } else {
                            Picker("Window", selection: $selectedWindowId) {
                                ForEach(windowList) { window in
                                    Text("\(window.appName): \(window.title)")
                                        .tag(window.windowId)
                                }
                            }
                            .pickerStyle(.menu)
                            .onChange(of: selectedWindowId) { _, newValue in
                                if let id = newValue {
                                    setup.captureTarget = .window(windowId: id)
                                }
                            }
                        }
                    }
                }
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
        .onAppear {
            Task { @MainActor in
                await fetchWindows()
            }
        }
    }

    private func fetchWindows() async {
        isFetchingWindows = true
        windowsError = nil
        defer { isFetchingWindows = false }

        do {
            let windows = try await IpcClientMock.captureWindows()
            windowList = windows
        } catch {
            windowsError = error
            // On error, fall back to display mode
            selectedChoice = .display
            setup.captureTarget = .display
            selectedWindowId = nil
        }
    }
}
