import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class AppViewModel: ObservableObject {
    enum State {
        case idle(audio: URL?)
        case working(meetingId: String, audioName: String)
        case done(notesPath: String)
        case error(kind: String, message: String)
    }

    @Published var state: State = .idle(audio: nil)

    private let client: IpcClient
    private var pollingTask: Task<Void, Never>?

    init(client: IpcClient) {
        self.client = client
    }

    func connect() async throws {
        try await client.connect()
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
            let meetingId = try await client.meetingStart()
            _ = try await client.notesGenerate(audio: audio, meetingId: meetingId)
            state = .working(meetingId: meetingId, audioName: audio.lastPathComponent)
            startPolling(meetingId: meetingId)
        } catch let IpcClientError.rpcError(_, message) {
            // RPC errors from the engine carry a curated, wire-safe
            // message (slice 05 hardening); pass them through.
            state = .error(kind: "rpc_error", message: message)
        } catch {
            // Internal client errors may contain socket paths, NWError
            // details, etc. — not safe for the UI. Log the full detail
            // to stderr (Console.app) and show an opaque label.
            Self.logFullError("notes.generate", error)
            state = .error(kind: "internal", message: "Could not reach the engine.")
        }
    }

    /// Log the full error to stderr (matches slice-05/07 wire-side
    /// sanitization pattern). Engine logs interleave with these via
    /// the shared FileHandle.standardError pipe.
    /// Non-isolated so detached tasks (polling loop) can call it.
    nonisolated static func logFullError(_ op: String, _ error: any Error) {
        let message = "panops-shell: \(op) failed: \(error)\n"
        FileHandle.standardError.write(Data(message.utf8))
    }

    private func startPolling(meetingId: String) {
        pollingTask?.cancel()
        let client = self.client
        // Run the polling loop on a detached task so the 2s sleep and
        // FileManager.fileExists checks don't occupy the @MainActor's
        // run loop. State updates hop back to MainActor explicitly.
        pollingTask = Task.detached { [weak self] in
            // Capture a weak MainActor-isolated reference for safe re-entry.
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
            let deadline = Date().addingTimeInterval(5 * 60) // 5 min ceiling
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
        // Defense in depth: the engine's `meeting.get` returns a
        // `dir_path` that should always live under panops's data dir
        // (~/Library/Application Support/panops/meetings/<uuid>/).
        // Validate before handing to NSWorkspace so a corrupted /
        // misconfigured engine response can't open Finder at an
        // arbitrary filesystem location.
        let panopsRoot = FileManager.default
            .homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/panops/")
            .standardizedFileURL
            .path
        let url = URL(fileURLWithPath: path).standardizedFileURL
        guard url.path.hasPrefix(panopsRoot) else {
            Self.logFullError(
                "reveal",
                NSError(
                    domain: "PanopsShell",
                    code: 1,
                    userInfo: [
                        NSLocalizedDescriptionKey:
                            "refusing to reveal path outside panops data dir: \(path)"
                    ]
                )
            )
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
