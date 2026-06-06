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

    init(client: IpcClient) {
        self.client = client
        self.eventStream = EventStreamActor()
    }

    func connect() async throws {
        try await client.connect()
    }

    /// Retry connection to engine after a previous failure.
    /// Used when state is `.engineNotConnected`.
    func retryConnect() async {
        do {
            try await client.connect()
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
                // Register callback for job completion (event-driven, no polling)
                await eventStream.registerCallback(jobId: jobId, handler: { [weak self] event in
                    Task { @MainActor in
                        switch event {
                        case .jobDone(_, let result):
                            self?.state = .done(notesPath: result.primaryFile)
                            // Refresh meetings list to show the new meeting
                            Task { await self?.refreshMeetings() }
                        case .jobError(_, let payload):
                            self?.state = .error(kind: payload.kind, message: payload.message)
                        case .unknown:
                            break
                        }
                    }
                })
            } else {
                // WebSocket failed — fall back to filesystem polling
                startPolling(meetingId: meetingId)
            }
        } catch let IpcClientError.rpcError(_, message) {
            state = .error(kind: "rpc_error", message: message)
        } catch {
            Self.logFullError("notes.generate", error)
            state = .error(kind: "internal", message: "Could not reach the engine.")
        }
    }

    /// Ensure WebSocket subscription is active. Lazy per spec decision.
    /// Returns true if WebSocket connected successfully, false on failure.
    private func ensureWsSubscription() async -> Bool {
        // Only subscribe once
        guard wsSubscriptionTask == nil else { return true }
        do {
            try await client.wsConnect()
            try await eventStream.subscribe(client: client)
            wsSubscriptionTask = Task {
                // EventStreamActor.subscribe handles the stream internally
            }
            return true
        } catch {
            Self.logFullError("ws.subscribe", error)
            // WebSocket failure is non-fatal; caller falls back to polling
            return false
        }
    }

    /// Fetch meeting list from engine. Called on app launch and refresh.
    func refreshMeetings() async {
        do {
            meetings = try await client.meetingList()
        } catch {
            Self.logFullError("meeting.list", error)
            // Keep existing meetings on error
        }
    }

    /// Load meeting detail when selected.
    func loadSelectedMeeting() async {
        guard let id = selectedMeetingId else {
            selectedMeeting = nil
            return
        }
        do {
            selectedMeeting = try await client.meetingGet(id: id)
        } catch {
            Self.logFullError("meeting.get", error)
            selectedMeeting = nil
        }
    }

    /// Log the full error to stderr.
    nonisolated static func logFullError(_ op: String, _ error: any Error) {
        let message = "panops-shell: \(op) failed: \(error)\n"
        FileHandle.standardError.write(Data(message.utf8))
    }

    private func startPolling(meetingId: String) {
        // Fallback polling if WebSocket isn't available
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
                    mainActorRef?.state = .error(kind: "internal", message: "Lost contact with the engine.")
                }
                return
            }
            let notesPath = (meeting.dirPath as NSString).appendingPathComponent("notes.md")
            let deadline = Date().addingTimeInterval(Self.pollingDeadlineSeconds)
            while !Task.isCancelled, Date() < deadline {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                if FileManager.default.fileExists(atPath: notesPath) {
                    await MainActor.run {
                        mainActorRef?.state = .done(notesPath: notesPath)
                    }
                    return
                }
            }
            await MainActor.run {
                guard let mainActorRef else { return }
                if case .working = mainActorRef.state {
                    mainActorRef.state = .error(kind: "timeout", message: "notes.generate did not complete within \(Int(Self.pollingDeadlineSeconds / 60)) minutes")
                }
            }
        }
    }

    func reveal(_ path: String) {
        let panopsRoot = FileManager.default
            .homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/panops/")
            .standardizedFileURL
            .path
        let url = URL(fileURLWithPath: path).standardizedFileURL
        guard url.path.hasPrefix(panopsRoot) else {
            Self.logFullError("reveal", NSError(domain: "PanopsShell", code: 1, userInfo: [NSLocalizedDescriptionKey: "refusing to reveal path outside panops data dir: \(path)"]))
            return
        }
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    func reset() {
        pollingTask?.cancel()
        wsSubscriptionTask?.cancel()
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
            if vm.selectedMeetingId != nil {
                Task { await vm.loadSelectedMeeting() }
            }
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