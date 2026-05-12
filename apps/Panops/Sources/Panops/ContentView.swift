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
            state = .error(kind: "rpc_error", message: message)
        } catch {
            state = .error(kind: "internal", message: "\(error)")
        }
    }

    private func startPolling(meetingId: String) {
        pollingTask?.cancel()
        pollingTask = Task { [weak self] in
            guard let self else { return }
            // Fetch the meeting once to learn its dir_path; subsequent
            // polls just stat the filesystem.
            let meeting: Meeting
            do {
                meeting = try await self.client.meetingGet(id: meetingId)
            } catch {
                await MainActor.run {
                    self.state = .error(kind: "internal", message: "meeting.get failed: \(error)")
                }
                return
            }
            let notesPath = (meeting.dirPath as NSString).appendingPathComponent("notes.md")
            let deadline = Date().addingTimeInterval(5 * 60) // 5 min ceiling
            while !Task.isCancelled, Date() < deadline {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                if FileManager.default.fileExists(atPath: notesPath) {
                    await MainActor.run {
                        self.state = .done(notesPath: notesPath)
                    }
                    return
                }
            }
            await MainActor.run {
                if case .working = self.state {
                    self.state = .error(kind: "timeout", message: "notes.generate did not complete within 5 minutes")
                }
            }
        }
    }

    func reveal(_ path: String) {
        let url = URL(fileURLWithPath: path)
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
