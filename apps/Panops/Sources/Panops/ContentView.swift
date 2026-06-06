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
    @Published var meetings: [MeetingSummary] = []
    @Published var selectedMeeting: Meeting?

    private let client: IpcClient
    private var pollingTask: Task<Void, Never>?
    private let eventStream: EventStreamActor
    private var wsSubscriptionTask: Task<Void, Never>?
    nonisolated private static let pollingDeadlineSeconds: TimeInterval = 5 * 60
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
        await refreshMeetingsWithStartupRetry()
    }

    /// Retry connection to engine after a previous failure.
    /// Used when state is `.engineNotConnected`.
    func retryConnect() async {
        do {
            try await client.connect()
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

    func generate() async {
        guard case .idle(let audio?) = state else { return }
        do {
            // Ensure WebSocket subscription is active (lazy)
            // If WebSocket fails, fall back to filesystem polling (fix #5)
            let wsOk = await ensureWsSubscription()

            let meetingId = try await client.meetingStart()
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
                            self?.finishGenerationError(
                                meetingId: meetingId,
                                jobId: jobId,
                                kind: payload.kind,
                                message: payload.message
                            )
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
            state = .error(kind: "rpc_error", message: message)
        } catch {
            Self.logFullError("notes.generate", error)
            state = .error(kind: "internal", message: "Could not reach the engine.")
        }
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
        pollingTask?.cancel()
        pollingTask = nil
        if let jobId {
            Task { await eventStream.unregisterCallback(jobId: jobId) }
        }
        state = .done(notesPath: notesPath)
        Task { await refreshMeetings() }
    }

    private func finishGenerationError(meetingId: String, jobId: String? = nil, kind: String, message: String) {
        guard case .working(let currentMeetingId, _) = state, currentMeetingId == meetingId else { return }
        pollingTask?.cancel()
        pollingTask = nil
        if let jobId {
            Task { await eventStream.unregisterCallback(jobId: jobId) }
        }
        state = .error(kind: kind, message: message)
    }

    private func startPolling(meetingId: String, jobId: String? = nil) {
        // Fallback polling if WebSocket isn't available, and safety-net polling
        // if WebSocket disconnects before a terminal event is delivered.
        pollingTask?.cancel()
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
            let deadline = Date().addingTimeInterval(Self.pollingDeadlineSeconds)
            while !Task.isCancelled, Date() < deadline {
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
            }
            await MainActor.run {
                mainActorRef?.finishGenerationError(
                    meetingId: meetingId,
                    jobId: jobId,
                    kind: "timeout",
                    message: "notes.generate did not complete within \(Int(Self.pollingDeadlineSeconds / 60)) minutes"
                )
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

struct ContentView: View {
    @ObservedObject var vm: AppViewModel
    @StateObject private var recordingController = MockRecordingController()

    var body: some View {
        NavigationSplitView {
            MeetingListView(vm: vm)
        } detail: {
            switch vm.state {
            case .engineNotConnected:
                engineNotConnectedView()
            case .idle(let audio):
                detailPlaceholder(audio: audio)
            case .working(_, let audioName):
                workingView(audioName: audioName)
            case .done(let path):
                doneView(path: path)
            case .error(let kind, let message):
                errorView(kind: kind, message: message)
            }
        }
        .frame(minWidth: 720, minHeight: 480)
        .task {
            await vm.refreshMeetings()
        }
        .onChange(of: vm.selectedMeetingId) { _, _ in
            Task { await vm.loadSelectedMeeting() }
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

    @ViewBuilder
    private func detailPlaceholder(audio: URL?) -> some View {
        VStack(spacing: 24) {
            // Show selected meeting detail if available
            if let meeting = vm.selectedMeeting {
                MeetingDetailView(meeting: meeting, recordingController: recordingController)
            } else {
                VStack(spacing: 16) {
                    Text("Panops").font(.largeTitle)
                    Text("Select a meeting from the sidebar or generate notes from an audio file.")
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)

                    Divider()

                    // Notes generation flow preserved here
                    HStack {
                        Button("Open audio…") { vm.pickAudio() }
                        if let audio {
                            VStack(alignment: .leading, spacing: 8) {
                                Text(audio.lastPathComponent).font(.body)
                                Button("Generate notes") {
                                    Task { await vm.generate() }
                                }
                                .keyboardShortcut(.return, modifiers: [])
                            }
                        } else {
                            Text("No file selected").foregroundStyle(.secondary)
                        }
                    }

                    Spacer()
                }
                .padding()
            }
        }
    }

    @ViewBuilder
    private func workingView(audioName: String) -> some View {
        VStack(spacing: 16) {
            HStack(spacing: 12) {
                ProgressView()
                Text("Generating notes…").font(.headline)
            }
            Text(audioName).foregroundStyle(.secondary)
            Spacer()
        }
        .padding()
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
