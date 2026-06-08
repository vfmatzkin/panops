import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class AppViewModel: ObservableObject {
    enum State {
        case engineNotConnected
        case idle(audio: URL?)
        case working(meetingId: String, audioName: String)
        case done(notesPath: String)
        case error(kind: String, message: String)
    }

    @Published var state: State = .idle(audio: nil)
    @Published var selectedMeetingId: String?
    @Published var activeRecordingMeetingId: String?
    @Published var meetings: [MeetingSummary] = []
    @Published var selectedMeeting: Meeting?
    @Published var notesProgress: JobProgressEvent?
    @Published var llmInfo: LlmInfo?
    /// Which meeting the current/last notes generation targets. Lets the meeting
    /// workspace show processing/error inline for the right meeting (the audio-
    /// file flow targets a freshly-created meeting that isn't selected, so its
    /// states render in the no-selection area instead).
    @Published var notesGenMeetingId: String?
    /// Bumped when notes generation completes so the open meeting workspace
    /// re-reads `notes.json` from disk.
    @Published private(set) var notesReloadTick: Int = 0

    private let client: IpcClient
    private var pollingTask: Task<Void, Never>?
    private let eventStream: EventStreamActor
    private var wsSubscriptionTask: Task<Void, Never>?
    private var notesLastProgressAt: Date?
    nonisolated private static let progressSilenceTimeoutSeconds: TimeInterval = 5 * 60
    nonisolated private static let wsSetupTimeoutNanoseconds: UInt64 = 3_000_000_000

    private enum WsSetupResult: Sendable {
        case succeeded
        case failed
        case timedOut
    }

    init(client: IpcClient) {
        self.client = client
        self.eventStream = EventStreamActor()
    }

    func connect() async throws {
        try await client.connect()
        await loadServerInfoBestEffort()
        await refreshMeetingsWithStartupRetry()
    }

    /// Retry connection to engine after a previous failure.
    /// Used when state is `.engineNotConnected`.
    func retryConnect() async {
        do {
            try await client.connect()
            await loadServerInfoBestEffort()
            await refreshMeetingsWithStartupRetry()
            state = .idle(audio: nil)
        } catch {
            Self.logFullError("ipc.connect.retry", error)
            // Stay in engineNotConnected state
        }
    }

    func setEngineNotConnected() {
        state = .engineNotConnected
    }

    func pickAudio() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowedContentTypes = [
            UTType.wav,
            UTType(filenameExtension: "m4a") ?? UTType.audio,
            UTType.mp3,
            UTType.movie,
        ]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        state = .idle(audio: url)
    }

    /// Audio-file flow: create a fresh meeting from a picked file and generate.
    func generate() async {
        guard case .idle(let audio?) = state else { return }
        do {
            let meetingId = try await client.meetingStart()
            await beginNotesGeneration(meetingId: meetingId, audio: audio)
        } catch let IpcClientError.rpcError(_, message) {
            failNotesStart(kind: "rpc_error", message: message)
        } catch {
            Self.logFullError("meeting.start", error)
            failNotesStart(kind: "internal", message: "Could not reach the engine.")
        }
    }

    /// Selected-meeting flow: generate (or retry) notes for an existing meeting
    /// using audio already captured in its directory. Targets the meeting's own
    /// id rather than creating a new one.
    func generateNotes(for meeting: Meeting) async {
        // Only one notes job is tracked at a time (notesGenMeetingId + state).
        // Starting a second while one runs would clobber the first's progress
        // and completion tracking, so refuse while any job is in flight.
        guard !isNotesJobActive else { return }
        guard let audio = locateAudio(in: meeting.dirPath) else {
            notesGenMeetingId = meeting.id
            notesProgress = nil
            notesLastProgressAt = nil
            state = .error(
                kind: "no_audio",
                message: "No recorded audio was found for this meeting."
            )
            return
        }
        await beginNotesGeneration(meetingId: meeting.id, audio: audio)
    }

    /// Shared notes-generation machinery used by both entry points: subscribe
    /// (or fall back to polling), kick off `notes.generate`, register the
    /// completion callback, and arm the polling safety net.
    private func beginNotesGeneration(meetingId: String, audio: URL) async {
        do {
            notesProgress = nil
            notesLastProgressAt = Date()
            notesGenMeetingId = meetingId

            // Ensure WebSocket subscription is active (lazy).
            // If WebSocket fails, fall back to filesystem polling (fix #5).
            let wsOk = await ensureWsSubscription()

            let jobId = try await client.notesGenerate(audio: audio, meetingId: meetingId)
            state = .working(meetingId: meetingId, audioName: audio.lastPathComponent)

            if wsOk {
                // Register callback for job completion (event-driven). Keep the
                // polling guard active too: if the WebSocket stream ends before
                // a terminal event arrives, the UI must not stay working forever.
                await eventStream.registerCallback(jobId: jobId, handler: { [weak self] event in
                    Task { @MainActor in
                        switch event {
                        case .jobDone(_, let result):
                            self?.finishGenerationDone(
                                meetingId: result.meetingId,
                                jobId: jobId,
                                notesPath: result.primaryFile
                            )
                        case .jobError(_, let payload):
                            self?.recordNotesProgressHeartbeat()
                            self?.finishGenerationError(
                                meetingId: meetingId,
                                jobId: jobId,
                                kind: payload.kind,
                                message: payload.message
                            )
                        case .jobProgress(let progress):
                            self?.updateNotesProgress(progress)
                        case .unknown:
                            break
                        }
                    }
                })
            }
            // Filesystem polling is also the WebSocket safety net. It is
            // cancelled by the terminal WebSocket callback when one arrives.
            startPolling(meetingId: meetingId, jobId: wsOk ? jobId : nil)
        } catch let IpcClientError.rpcError(_, message) {
            failNotesStart(kind: "rpc_error", message: message)
        } catch {
            Self.logFullError("notes.generate", error)
            failNotesStart(kind: "internal", message: "Could not reach the engine.")
        }
    }

    private func failNotesStart(kind: String, message: String) {
        notesProgress = nil
        notesLastProgressAt = nil
        // Keep notesGenMeetingId so the workspace can show the error inline.
        state = .error(kind: kind, message: message)
    }

    /// Find captured audio in a meeting directory. Live capture writes
    /// `system.wav` / `mic.wav`; prefer those, then any audio-like file.
    private func locateAudio(in dirPath: String) -> URL? {
        let fm = FileManager.default
        let preferred = ["system.wav", "mic.wav", "audio.wav"]
        for name in preferred {
            let path = (dirPath as NSString).appendingPathComponent(name)
            if PathValidator.isPath(path, under: dirPath), fm.fileExists(atPath: path) {
                return URL(fileURLWithPath: path)
            }
        }
        let audioExtensions: Set<String> = ["wav", "m4a", "mp3", "mov"]
        if let contents = try? fm.contentsOfDirectory(atPath: dirPath) {
            for name in contents.sorted()
            where audioExtensions.contains((name as NSString).pathExtension.lowercased()) {
                let path = (dirPath as NSString).appendingPathComponent(name)
                if PathValidator.isPath(path, under: dirPath) {
                    return URL(fileURLWithPath: path)
                }
            }
        }
        return nil
    }

    /// True while any notes-generation job is in flight. Only one job is tracked
    /// at a time, so callers must not start a second while this is true.
    var isNotesJobActive: Bool {
        if case .working = state { return true }
        return false
    }

    /// Lifecycle status for a meeting summary, used by the sidebar status pill.
    func status(for summary: MeetingSummary) -> MeetingStatus {
        if activeRecordingMeetingId == summary.id { return .recording }
        if notesGenMeetingId == summary.id, case .working = state { return .processing }
        if summary.hasNotes { return .ready }
        if summary.endedAt != nil { return .needsNotes }
        // Older payloads (pre ended_at) decode endedAt as nil; a positive
        // recorded duration still means the meeting ended and needs notes.
        if summary.durationMs > 0 { return .needsNotes }
        return .draft
    }

    /// Delete a meeting via the existing `meeting.delete` path; clear selection
    /// if it was the open one, then refresh the list.
    func deleteMeeting(id: String) async {
        do {
            try await client.meetingDelete(id: id)
            // Only clear the selection / empty the workspace once the delete
            // actually succeeded; a failed delete must leave the meeting open.
            if selectedMeetingId == id {
                selectedMeetingId = nil
                selectedMeeting = nil
                state = .idle(audio: nil)
            }
        } catch {
            Self.logFullError("meeting.delete", error)
        }
        await refreshMeetings()
    }

    /// Open a path (typically a meeting directory) in Finder, guarded to the
    /// panops data dir.
    func openInFinder(path: String) {
        guard PathValidator.isUnderPanopsDataDir(path) else {
            Self.logFullError(
                "openInFinder",
                NSError(domain: "PanopsShell", code: 1, userInfo: [NSLocalizedDescriptionKey: "refusing to open path outside panops data dir: \(path)"])
            )
            return
        }
        NSWorkspace.shared.open(URL(fileURLWithPath: path).standardizedFileURL)
    }

    /// Ensure WebSocket subscription is active. Lazy per spec decision.
    /// Returns true if WebSocket connected successfully, false on failure or
    /// timeout so the caller can start filesystem polling instead of hanging.
    private func ensureWsSubscription() async -> Bool {
        // Only subscribe once
        guard wsSubscriptionTask == nil else { return true }

        switch await Self.runWsSetupWithTimeout(client: client, eventStream: eventStream) {
        case .succeeded:
            wsSubscriptionTask = Task {
                // EventStreamActor.subscribe handles the stream internally
            }
            return true
        case .failed:
            // WebSocket failure is non-fatal; caller falls back to polling
            return false
        case .timedOut:
            Self.logFullError(
                "ws.subscribe",
                IpcClientError.websocketUpgradeFailed("WebSocket setup timed out")
            )
            // WebSocket stall is non-fatal; caller falls back to polling
            return false
        }
    }

    /// Race WebSocket setup against a short timer. This intentionally uses
    /// unstructured tasks rather than a task group because a stalled Network
    /// continuation may ignore cancellation; the timeout must still let the UI
    /// fall through to polling.
    nonisolated private static func runWsSetupWithTimeout(
        client: IpcClient,
        eventStream: EventStreamActor
    ) async -> WsSetupResult {
        let stream = AsyncStream<WsSetupResult> { continuation in
            let setupTask = Task {
                do {
                    try await client.wsConnect()
                    try Task.checkCancellation()
                    try await eventStream.subscribe(client: client)
                    continuation.yield(.succeeded)
                } catch {
                    Self.logFullError("ws.subscribe", error)
                    continuation.yield(.failed)
                }
                continuation.finish()
            }

            let timeoutTask = Task {
                try? await Task.sleep(nanoseconds: Self.wsSetupTimeoutNanoseconds)
                guard !Task.isCancelled else { return }
                setupTask.cancel()
                await eventStream.stop()
                await client.disconnect()
                continuation.yield(.timedOut)
                continuation.finish()
            }

            continuation.onTermination = { @Sendable _ in
                setupTask.cancel()
                timeoutTask.cancel()
            }
        }

        for await result in stream {
            return result
        }
        return .timedOut
    }

    /// Fetch engine status once after IPC connection. Best-effort: older or
    /// unhealthy engines simply omit the chip instead of blocking the app.
    private func loadServerInfoBestEffort() async {
        do {
            let info = try await client.serverInfo()
            llmInfo = info.llm
        } catch {
            Self.logFullError("ipc.server.info", error)
            llmInfo = nil
        }
    }

    /// Fetch meeting list from engine. Called on app launch and refresh.
    func refreshMeetings() async {
        await refreshMeetings(maxAttempts: 1, initialDelayMs: 0)
    }

    /// Fetch meeting list during startup after IPC connects. The launch-time
    /// ContentView task can race engine bootstrap; this retry ensures a
    /// transient not-ready engine does not leave the sidebar empty forever.
    private func refreshMeetingsWithStartupRetry() async {
        await refreshMeetings(maxAttempts: 4, initialDelayMs: 200)
    }

    private func refreshMeetings(maxAttempts: Int, initialDelayMs: UInt64) async {
        var delayMs = initialDelayMs
        for attempt in 1...maxAttempts {
            do {
                meetings = try await client.meetingList()
                return
            } catch {
                Self.logFullError("meeting.list", error)
                guard attempt < maxAttempts else { return }
                if delayMs > 0 {
                    try? await Task.sleep(nanoseconds: delayMs * 1_000_000)
                    delayMs = min(delayMs * 2, 1_000)
                }
            }
        }
    }

    /// Create a meeting through the existing `ipc.meeting.start` path, start
    /// live capture for it, select it in the sidebar, and refresh the list.
    /// `setup` carries the New Recording sheet's choices: title + language flow
    /// into `meeting.start`, audio sources + screenshot sampling into
    /// `recording.start`.
    func startNewRecording<Controller: RecordingController>(
        using recordingController: Controller,
        setup: RecordingSetup = .default
    ) async throws {
        let meetingId = try await client.meetingStart(config: setup.meetingConfig)
        do {
            try await recordingController.start(meetingId: meetingId, options: setup.recordingOptions)
        } catch {
            let recordingStartError = error
            do {
                // No recording was accepted, so remove the provisional row and
                // meeting directory rather than leaving a bogus open meeting.
                try await client.meetingDelete(id: meetingId)
            } catch {
                Self.logFullError("meeting.delete.cleanup", error)
            }
            await refreshMeetings()
            throw recordingStartError
        }

        activeRecordingMeetingId = meetingId
        selectedMeetingId = meetingId
        selectedMeeting = nil
        state = .idle(audio: nil)
        await refreshMeetings()
        await loadSelectedMeeting()
    }

    /// Load meeting detail when selected.
    func loadSelectedMeeting() async {
        guard let id = selectedMeetingId else {
            selectedMeeting = nil
            return
        }
        do {
            let meeting = try await client.meetingGet(id: id)
            guard selectedMeetingId == id else { return }
            selectedMeeting = meeting
            showSelectedMeetingAfterTerminalState()
        } catch {
            Self.logFullError("meeting.get", error)
            selectedMeeting = nil
        }
    }

    /// Close the meeting row after a live recording stops, then refresh list
    /// and detail so ended_at/duration_ms are visible immediately.
    func finishLiveRecording(meetingId: String) async throws {
        let stoppedMeeting = try await client.meetingStop(id: meetingId)
        if activeRecordingMeetingId == meetingId {
            activeRecordingMeetingId = nil
        }
        await refreshMeetings()
        if selectedMeetingId == meetingId {
            selectedMeeting = stoppedMeeting
            state = .idle(audio: nil)
        }
    }

    /// Close whichever meeting owns the active recording, regardless of the
    /// sidebar selection when the user presses Stop.
    func finishActiveLiveRecording() async throws {
        guard let meetingId = activeRecordingMeetingId else { return }
        try await finishLiveRecording(meetingId: meetingId)
    }

    private func showSelectedMeetingAfterTerminalState() {
        switch state {
        case .done, .error:
            state = .idle(audio: nil)
        case .engineNotConnected, .idle, .working:
            break
        }
    }

    /// Log the full error to stderr.
    nonisolated static func logFullError(_ op: String, _ error: any Error) {
        let message = "panops-shell: \(op) failed: \(error)\n"
        FileHandle.standardError.write(Data(message.utf8))
    }

    private func finishGenerationDone(meetingId: String, jobId: String? = nil, notesPath: String) {
        guard case .working(let currentMeetingId, _) = state, currentMeetingId == meetingId else { return }
        recordNotesProgressHeartbeat()
        pollingTask?.cancel()
        pollingTask = nil
        if let jobId {
            Task { await eventStream.unregisterCallback(jobId: jobId) }
        }
        notesProgress = nil
        notesLastProgressAt = nil
        notesGenMeetingId = nil
        // Signal the open meeting workspace to re-read notes.json from disk.
        notesReloadTick += 1
        state = .done(notesPath: notesPath)
        Task { await refreshMeetings() }
    }

    private func finishGenerationError(meetingId: String, jobId: String? = nil, kind: String, message: String) {
        guard case .working(let currentMeetingId, _) = state, currentMeetingId == meetingId else { return }
        recordNotesProgressHeartbeat()
        pollingTask?.cancel()
        pollingTask = nil
        if let jobId {
            Task { await eventStream.unregisterCallback(jobId: jobId) }
        }
        notesProgress = nil
        notesLastProgressAt = nil
        state = .error(kind: kind, message: message)
    }

    private func updateNotesProgress(_ progress: JobProgressEvent) {
        notesProgress = progress
        recordNotesProgressHeartbeat()
    }

    private func recordNotesProgressHeartbeat() {
        notesLastProgressAt = Date()
    }

    private func notesProgressStalledMessage() -> String {
        let minutes = Int(Self.progressSilenceTimeoutSeconds / 60)
        return "notes.generate stalled: no progress for \(minutes) minutes"
    }

    private func startPolling(meetingId: String, jobId: String? = nil) {
        // Fallback polling if WebSocket isn't available, and safety-net polling
        // if WebSocket disconnects before a terminal event is delivered.
        pollingTask?.cancel()
        notesLastProgressAt = Date()
        let client = self.client
        pollingTask = Task.detached { [weak self] in
            let mainActorRef = self
            let meeting: Meeting
            do {
                meeting = try await client.meetingGet(id: meetingId)
            } catch {
                Self.logFullError("meeting.get", error)
                await MainActor.run {
                    mainActorRef?.finishGenerationError(
                        meetingId: meetingId,
                        jobId: jobId,
                        kind: "internal",
                        message: "Lost contact with the engine."
                    )
                }
                return
            }
            let notesPath = (meeting.dirPath as NSString).appendingPathComponent("notes.md")
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                if FileManager.default.fileExists(atPath: notesPath) {
                    await MainActor.run {
                        mainActorRef?.finishGenerationDone(
                            meetingId: meetingId,
                            jobId: jobId,
                            notesPath: notesPath
                        )
                    }
                    return
                }
                let hasStalled = await MainActor.run { () -> Bool in
                    let lastProgressAt = mainActorRef?.notesLastProgressAt ?? Date()
                    return Date().timeIntervalSince(lastProgressAt) >= Self.progressSilenceTimeoutSeconds
                }
                if hasStalled {
                    await MainActor.run {
                        mainActorRef?.finishGenerationError(
                            meetingId: meetingId,
                            jobId: jobId,
                            kind: "timeout",
                            message: mainActorRef?.notesProgressStalledMessage()
                                ?? "notes.generate stalled: no progress"
                        )
                    }
                    return
                }
            }
        }
    }

    func reveal(_ path: String) {
        guard PathValidator.isUnderPanopsDataDir(path) else {
            Self.logFullError("reveal", NSError(domain: "PanopsShell", code: 1, userInfo: [NSLocalizedDescriptionKey: "refusing to reveal path outside panops data dir: \(path)"]))
            return
        }
        let url = URL(fileURLWithPath: path).standardizedFileURL
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    func reset() {
        pollingTask?.cancel()
        pollingTask = nil
        wsSubscriptionTask?.cancel()
        wsSubscriptionTask = nil
        Task { await eventStream.stop() }
        selectedMeetingId = nil
        activeRecordingMeetingId = nil
        selectedMeeting = nil
        notesProgress = nil
        notesLastProgressAt = nil
        notesGenMeetingId = nil
        state = .idle(audio: nil)
    }

    /// Return from a browsed meeting detail to the audio-file generation flow.
    func startNewGenerationFlow() {
        selectedMeetingId = nil
        selectedMeeting = nil
        state = .idle(audio: nil)
    }

    func shutdown(engine: EngineProcess?) async {
        pollingTask?.cancel()
        wsSubscriptionTask?.cancel()
        await eventStream.stop()
        await client.disconnect()
        await engine?.stop()
    }
}

struct LlmProviderChip: View {
    let info: LlmInfo

    private var label: String {
        if info.local {
            return "Local · \(info.provider)/\(info.model)"
        }
        return "⚠︎ Cloud · \(info.provider)/\(info.model)"
    }

    private var tint: Color {
        info.local ? Color.secondary : Color.orange
    }

    private var fill: Color {
        info.local ? Color.secondary.opacity(0.12) : Color.orange.opacity(0.15)
    }

    var body: some View {
        Text(label)
            .font(.caption)
            .lineLimit(1)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .foregroundStyle(tint)
            .background(Capsule().fill(fill))
            .overlay(Capsule().stroke(tint.opacity(0.35), lineWidth: 1))
            .help(label)
    }
}

struct ContentView<Controller: RecordingController & ObservableObject>: View {
    @ObservedObject var vm: AppViewModel
    @ObservedObject var recordingController: Controller
    @State private var isStartingNewRecording = false
    @State private var toolbarRecordingError: String?
    @State private var showNewRecordingSheet = false
    /// The setup chosen in the sheet, kept so the recording screen can show the
    /// right capture-source indicators while recording.
    @State private var activeSetup = RecordingSetup.default

    var body: some View {
        NavigationSplitView {
            MeetingListView(vm: vm)
        } detail: {
            // Active recording takes over the detail pane with the dedicated
            // recording screen (timer + capture indicators + Stop), regardless
            // of sidebar selection.
            if recordingController.isRecording {
                RecordingScreen(
                    controller: recordingController,
                    setup: activeSetup,
                    onRecordingStopped: { _ in
                        try await vm.finishActiveLiveRecording()
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
                        }
                    },
                    onRecordingStopped: { _ in
                        try await vm.finishActiveLiveRecording()
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
        .frame(minWidth: 720, minHeight: 480)
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
                onStart: { setup in
                    showNewRecordingSheet = false
                    activeSetup = setup
                    Task { @MainActor in await startNewRecording(setup: setup) }
                },
                onCancel: { showNewRecordingSheet = false }
            )
        }
        .task {
            await vm.refreshMeetings()
        }
        .onChange(of: vm.selectedMeetingId) { _, _ in
            Task { await vm.loadSelectedMeeting() }
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
