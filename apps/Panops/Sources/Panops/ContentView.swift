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

    private let client: IpcClient
    private var pollingTask: Task<Void, Never>?
    private let eventStream: EventStreamActor
    private var wsSubscriptionTask: Task<Void, Never>?

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
            await ensureWsSubscription()

            let meetingId = try await client.meetingStart()
            let jobId = try await client.notesGenerate(audio: audio, meetingId: meetingId)
            state = .working(meetingId: meetingId, audioName: audio.lastPathComponent)

            // Register callback for job completion (event-driven, no polling)
            await eventStream.registerCallback(jobId: jobId, handler: { [weak self] event in
                Task { @MainActor in
                    switch event {
                    case .jobDone(_, let result):
                        self?.state = .done(notesPath: result.primaryFile)
                    case .jobError(_, let payload):
                        self?.state = .error(kind: payload.kind, message: payload.message)
                    case .unknown:
                        break
                    }
                }
            })
        } catch let IpcClientError.rpcError(_, message) {
            state = .error(kind: "rpc_error", message: message)
        } catch {
            Self.logFullError("notes.generate", error)
            state = .error(kind: "internal", message: "Could not reach the engine.")
        }
    }

    /// Ensure WebSocket subscription is active. Lazy per spec decision.
    private func ensureWsSubscription() async {
        // Only subscribe once
        guard wsSubscriptionTask == nil else { return }
        do {
            try await client.wsConnect()
            try await eventStream.subscribe(client: client)
            wsSubscriptionTask = Task {
                // EventStreamActor.subscribe handles the stream internally
            }
        } catch {
            Self.logFullError("ws.subscribe", error)
            // WebSocket failure is non-fatal; fall back to polling
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
            let deadline = Date().addingTimeInterval(5 * 60)
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
                    mainActorRef.state = .error(kind: "timeout", message: "notes.generate did not complete within 5 minutes")
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

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Panops").font(.largeTitle)
            Divider()
            switch vm.state {
            case .engineNotConnected:
                engineNotConnectedSection()
            case .idle(let audio):
                idleSection(audio: audio)
            case .working(_, let audioName):
                workingSection(audioName: audioName)
            case .done(let path):
                doneSection(path: path)
            case .error(let kind, let message):
                errorSection(kind: kind, message: message)
            }
            Spacer()
        }
        .padding()
        .frame(minWidth: 520, minHeight: 320)
        .task {
            await vm.refreshMeetings()
        }
    }

    @ViewBuilder
    private func idleSection(audio: URL?) -> some View {
        HStack {
            Button("Open audio…") { vm.pickAudio() }
            if let audio {
                Text(audio.lastPathComponent).font(.body)
                Spacer()
                Button("Generate notes") {
                    Task { await vm.generate() }
                }
                .keyboardShortcut(.return, modifiers: [])
            } else {
                Text("No file selected").foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func engineNotConnectedSection() -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Engine not connected").font(.headline).foregroundStyle(.orange)
            Text("Could not connect to the panops engine. Ensure panops-engine is running.")
            Button("Retry") {
                Task { await vm.retryConnect() }
            }
        }
    }

    @ViewBuilder
    private func workingSection(audioName: String) -> some View {
        HStack(spacing: 12) {
            ProgressView()
            Text("Working… (\(audioName))").foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private func doneSection(path: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Done").font(.headline).foregroundStyle(.green)
            Text(path).textSelection(.enabled).font(.system(.body, design: .monospaced))
            HStack {
                Button("Open in Finder") { vm.reveal(path) }
                Button("New") { vm.reset() }
            }
        }
    }

    @ViewBuilder
    private func errorSection(kind: String, message: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Error: \(kind)").font(.headline).foregroundStyle(.red)
            Text(message).textSelection(.enabled)
            Button("Try again") { vm.reset() }
        }
    }
}