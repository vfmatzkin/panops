import SwiftUI

/// Radio choice for the capture-target picker. File-scope (not nested) so the
/// pure `CaptureTargetResolver` and its tests can reference it.
enum CaptureTargetChoice: String, CaseIterable, Identifiable {
    case display = "Full display"
    case window = "Window…"

    var id: String { rawValue }
}

/// Pure mapping from the sheet's capture-target UI state to the `CaptureTarget`
/// submitted to `recording.start`. Extracted from the View so the
/// window-selection guard is unit-testable without driving SwiftUI.
enum CaptureTargetResolver {
    /// Sentinel target used while "Window…" is chosen but no real window is
    /// selected yet. The Start guard treats it as not-submittable so window_id 0
    /// never reaches the engine.
    static let unselectedWindow: CaptureTarget = .window(windowId: 0)

    /// Resolve the capture target from the radio choice, the selected window id,
    /// and the currently-available window list:
    /// - `.display` choice → `.display`.
    /// - `.window` with a real selected id present in the list → `.window(id)`.
    /// - `.window` with no windows available → `.display` (fall back).
    /// - `.window` with windows available but none chosen → `unselectedWindow`.
    static func resolve(
        choice: CaptureTargetChoice,
        selectedWindowId: UInt32?,
        windowList: [WindowInfo]
    ) -> CaptureTarget {
        switch choice {
        case .display:
            return .display
        case .window:
            if let id = selectedWindowId,
               id != 0,
               windowList.contains(where: { $0.windowId == id }) {
                return .window(windowId: id)
            }
            if windowList.isEmpty {
                return .display
            }
            return unselectedWindow
        }
    }

    /// A target is submittable unless it's the awaiting-a-pick sentinel.
    static func canStart(target: CaptureTarget) -> Bool {
        target != unselectedWindow
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
    /// Window-list provider. Production injects the real engine IPC
    /// (`IpcClient.captureWindows()` via the view model); tests/previews inject a
    /// fake list so they never touch a socket. `@MainActor` so it can safely
    /// reach the main-actor view model.
    let fetchWindows: @MainActor () async throws -> [WindowInfo]

    init(
        onStart: @escaping (RecordingSetup) -> Void,
        onCancel: @escaping () -> Void,
        fetchWindows: @escaping @MainActor () async throws -> [WindowInfo]
    ) {
        self.onStart = onStart
        self.onCancel = onCancel
        self.fetchWindows = fetchWindows
    }

    @State private var setup = RecordingSetup.default
    @State private var windowList: [WindowInfo] = []
    @State private var isFetchingWindows = false
    @State private var windowsError: Error?
    @State private var selectedWindowId: UInt32?
    @State private var selectedChoice: CaptureTargetChoice = .display

    /// Start is blocked only when the chosen target is the awaiting-a-pick
    /// sentinel; the display and a real window both submit.
    private var canStart: Bool {
        CaptureTargetResolver.canStart(target: setup.captureTarget)
    }

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
                    if newValue == .display { selectedWindowId = nil }
                    reconcileCaptureTarget()
                }

                if selectedChoice == .window {
                    windowSelection
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
                    .disabled(!canStart)
            }
            .padding(20)
        }
        .frame(width: 420)
        .onAppear {
            Task { @MainActor in
                await loadWindows()
            }
        }
    }

    /// The window-selection block shown under "Window…": a loading state, a
    /// fall-back note + retry when no windows exist (or the fetch failed), or the
    /// window menu when windows are available.
    @ViewBuilder
    private var windowSelection: some View {
        VStack(alignment: .leading, spacing: 8) {
            if isFetchingWindows {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Loading windows…")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } else if windowList.isEmpty {
                // No windows (none available, or the fetch failed): the resolver
                // has already fallen back to the full display, so Start stays
                // enabled. Offer a retry.
                HStack {
                    Text(windowsError == nil
                        ? "No windows available — recording the full display instead."
                        : "Couldn't load windows — recording the full display instead.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Try again") {
                        Task { @MainActor in await loadWindows() }
                    }
                    .font(.caption)
                }
            } else {
                Text("Select a window to capture.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Picker("Window", selection: $selectedWindowId) {
                    Text("Select a window…").tag(UInt32?.none)
                    ForEach(windowList) { window in
                        Text("\(window.appName): \(window.title)")
                            .tag(UInt32?.some(window.windowId))
                    }
                }
                .pickerStyle(.menu)
                .onChange(of: selectedWindowId) { _, _ in
                    reconcileCaptureTarget()
                }
            }
        }
    }

    /// Recompute `setup.captureTarget` from the current UI state. Keeps Start
    /// gated so a window target is only submitted with a real, selected window
    /// (never window_id 0); falls back to the full display when no windows exist.
    @MainActor
    private func reconcileCaptureTarget() {
        setup.captureTarget = CaptureTargetResolver.resolve(
            choice: selectedChoice,
            selectedWindowId: selectedWindowId,
            windowList: windowList
        )
    }

    /// Fetch the capturable window list from the injected provider (the real
    /// engine IPC in production). On failure or an empty list, fall back to the
    /// full display so the sheet stays usable and never submits window_id 0.
    @MainActor
    private func loadWindows() async {
        isFetchingWindows = true
        windowsError = nil
        defer { isFetchingWindows = false }

        do {
            windowList = try await fetchWindows()
        } catch {
            windowsError = error
            windowList = []
        }
        // Drop a stale selection no longer in the refreshed list, then recompute
        // the target (which falls back to display when the list is empty).
        if let id = selectedWindowId, !windowList.contains(where: { $0.windowId == id }) {
            selectedWindowId = nil
        }
        reconcileCaptureTarget()
    }
}
