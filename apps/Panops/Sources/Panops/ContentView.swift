import SwiftUI

struct ContentView<Controller: RecordingController & ObservableObject>: View {
    @ObservedObject var vm: AppViewModel
    @ObservedObject var recordingController: Controller
    /// The app's own capture preview, shared by the New Recording sheet and the
    /// recording screen so the preview + audio meters stay live across the
    /// hand-off from setup to recording.
    @StateObject private var preview = CapturePreviewController()
    @State private var isStartingNewRecording = false
    @State private var toolbarRecordingError: String?
    @State private var showNewRecordingSheet = false
    /// The setup chosen in the sheet, kept so the recording screen can show the
    /// right capture-source indicators while recording.
    @State private var activeSetup = RecordingSetup.default

    var body: some View {
        NavigationSplitView {
            OrgSidebarView(vm: vm)
        } content: {
            MeetingListView(vm: vm)
        } detail: {
            // Active recording takes over the detail pane with the dedicated
            // recording screen (timer + capture indicators + Stop), regardless
            // of sidebar selection.
            if recordingController.isRecording {
                RecordingScreen(
                    controller: recordingController,
                    preview: preview,
                    setup: activeSetup,
                    meetingDirPath: vm.activeRecordingDirPath,
                    onRecordingStopped: { outcome in
                        // The controller already cleared isRecording, so this
                        // RecordingScreen unmounts the instant stop succeeds and
                        // its local alert can never be seen. Route a finalize
                        // failure to ContentView's own error state, which stays
                        // mounted, so the user actually learns stop/finalize
                        // failed instead of silently dropping back to the list.
                        do {
                            try await vm.finishActiveLiveRecording(outcome: outcome)
                        } catch {
                            AppViewModel.logFullError("recording.finish", error)
                            toolbarRecordingError = "Recording stopped, but finishing the meeting failed."
                        }
                    }
                )
            } else if let meeting = vm.selectedMeeting {
                // A selected meeting owns its own Notes/Transcript/Info
                // workspace, including per-meeting processing/error states. The
                // audio-file flow (no selection) renders its working/done/error
                // here instead.
                MeetingDetailView(
                    meeting: meeting,
                    vm: vm,
                    recordingController: recordingController,
                    onRecordingStarted: { id in
                        await MainActor.run {
                            vm.activeRecordingMeetingId = id
                            // Pin this meeting's dir for the health-line poll.
                            vm.activeRecordingDirPath = meeting.dirPath
                        }
                    },
                    onRecordingStopped: { outcome in
                        try await vm.finishActiveLiveRecording(outcome: outcome)
                    }
                )
            } else {
                switch vm.state {
                case .engineNotConnected:
                    engineNotConnectedView()
                case .idle(let audio):
                    emptyState(audio: audio)
                case .working(_, let audioName):
                    workingView(audioName: audioName)
                case .done(let path):
                    doneView(path: path)
                case .error(let kind, let message):
                    errorView(kind: kind, message: message)
                }
            }
        }
        .frame(minWidth: 900, minHeight: 480)
        .toolbar {
            ToolbarItemGroup {
                if let llmInfo = vm.llmInfo {
                    LlmProviderChip(info: llmInfo)
                }

                if vm.selectedMeeting != nil {
                    Button("New") {
                        vm.startNewGenerationFlow()
                    }
                    .help("Start a new notes-generation flow")
                }

                Button("New Recording") {
                    showNewRecordingSheet = true
                }
                .disabled(isStartingNewRecording || recordingController.isRecording || isEngineNotConnected || isGeneratingNotes)
                .help("Create a meeting and start live recording")
            }
        }
        .alert("Recording error", isPresented: toolbarRecordingErrorPresented) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(toolbarRecordingError ?? "")
        }
        .sheet(isPresented: $showNewRecordingSheet) {
            NewRecordingSheet(
                preview: preview,
                onStart: { setup in
                    showNewRecordingSheet = false
                    activeSetup = setup
                    Task { @MainActor in await startNewRecording(setup: setup) }
                },
                onCancel: {
                    showNewRecordingSheet = false
                    preview.teardown()
                }
            )
        }
        .task {
            await vm.refreshMeetings()
        }
        .onChange(of: vm.selectedMeetingId) { _, _ in
            Task { await vm.loadSelectedMeeting() }
        }
        .onChange(of: vm.sidebarSelection) { _, _ in
            Task { await vm.applySidebarSelection() }
        }
        .onChange(of: recordingController.isRecording) { _, recording in
            // Keep the invariant "idle ⟹ activeSetup == .default". The sheet
            // overrides it just before a sheet-driven start; every other start
            // (the RecordBar resume) uses engine defaults, so once a recording
            // ends we reset here. That way the next non-sheet start already has
            // accurate capture chips before its RecordingScreen can appear,
            // instead of showing a stale setup from a prior sheet run.
            if !recording {
                activeSetup = .default
                // The recording ended (or never started) — stop the shared
                // preview stream so it doesn't keep capturing in the background.
                preview.teardown()
            }
        }
    }

    private var isEngineNotConnected: Bool {
        if case .engineNotConnected = vm.state {
            return true
        }
        return false
    }

    private var isGeneratingNotes: Bool {
        if case .working = vm.state {
            return true
        }
        return false
    }

    private var toolbarRecordingErrorPresented: Binding<Bool> {
        Binding(
            get: { toolbarRecordingError != nil },
            set: { if !$0 { toolbarRecordingError = nil } }
        )
    }

    private func startNewRecording(setup: RecordingSetup) async {
        guard !isStartingNewRecording else { return }
        isStartingNewRecording = true
        defer { isStartingNewRecording = false }

        do {
            toolbarRecordingError = nil
            try await vm.startNewRecording(using: recordingController, setup: setup)
        } catch {
            AppViewModel.logFullError("recording.new", error)
            toolbarRecordingError = "Couldn't start recording."
            // A failed sheet start may never flip isRecording (e.g. meeting.start
            // threw), so the isRecording reset can't fire — clear the chosen
            // setup here too so a later non-sheet start doesn't inherit it, and
            // stop the preview stream that the sheet left running.
            activeSetup = .default
            preview.teardown()
        }
    }

    @ViewBuilder
    private func engineNotConnectedView() -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Engine not connected")
                .font(.title2)
                .foregroundStyle(.orange)
            Text("Could not connect to the panops engine. Ensure panops-engine is running.")
            Button("Retry") {
                Task { await vm.retryConnect() }
            }
            Spacer()
        }
        .padding()
    }

    /// No meeting selected: product blurb, the primary New Recording CTA, the
    /// honest trust strip, and a secondary audio-file generation path.
    @ViewBuilder
    private func emptyState(audio: URL?) -> some View {
        VStack(spacing: 20) {
            Spacer()
            Image(systemName: "waveform.and.mic")
                .font(.system(size: 44))
                .foregroundStyle(Color.accentColor)
            Text("Panops").font(.largeTitle.weight(.semibold))
            Text("Record a meeting and get private, screenshot-anchored notes —\nall processed on this Mac.")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            Button {
                showNewRecordingSheet = true
            } label: {
                Label("New Recording", systemImage: "record.circle")
                    .padding(.horizontal, 8)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(isStartingNewRecording || recordingController.isRecording || isEngineNotConnected)

            // Secondary path: generate notes from an existing audio file.
            VStack(spacing: 8) {
                Divider().frame(maxWidth: 280)
                HStack(spacing: 12) {
                    Button("Open audio file…") { vm.pickAudio() }
                    if let audio {
                        Text(audio.lastPathComponent)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Button("Generate notes") {
                            Task { await vm.generate() }
                        }
                        .keyboardShortcut(.return, modifiers: [])
                    }
                }
            }
            .padding(.top, 4)

            Spacer()
            TrustStrip()
                .padding(.bottom, 12)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
    }

    @ViewBuilder
    private func workingView(audioName: String) -> some View {
        let progress = vm.notesProgress
        VStack(spacing: 16) {
            VStack(spacing: 8) {
                if let progress,
                   let current = progress.current,
                   let total = progress.total,
                   total > 0 {
                    ProgressView(value: max(0.0, min(Double(current) / Double(total), 1.0)))
                        .frame(maxWidth: 280)
                } else {
                    ProgressView()
                }

                Text(notesProgressLabel(progress)).font(.headline)
                if let message = progress?.message, !message.isEmpty {
                    Text(message)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }
            Text(audioName).foregroundStyle(.secondary)
            Spacer()
        }
        .padding()
    }

    private func notesProgressLabel(_ progress: JobProgressEvent?) -> String {
        progress?.stageLabel ?? "Generating notes…"
    }

    @ViewBuilder
    private func doneView(path: String) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Done").font(.title2).foregroundStyle(.green)
            Text(path).textSelection(.enabled).font(.system(.body, design: .monospaced))
            HStack {
                Button("Open in Finder") { vm.reveal(path) }
                Button("New") { vm.reset() }
            }
            Spacer()
        }
        .padding()
    }

    @ViewBuilder
    private func errorView(kind: String, message: String) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Error: \(kind)").font(.title2).foregroundStyle(.red)
            Text(message).textSelection(.enabled)
            Button("Try again") { vm.reset() }
            Spacer()
        }
        .padding()
    }
}
