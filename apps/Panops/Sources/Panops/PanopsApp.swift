import SwiftUI

@main
struct PanopsApp: App {
    @StateObject private var viewModel: AppViewModel
    @State private var engine: EngineProcess?
    @State private var startupError: String?

    init() {
        let socketPath = FileManager.default
            .homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/panops/engine.sock")
        let client = IpcClient(socketPath: socketPath)
        self._viewModel = StateObject(wrappedValue: AppViewModel(client: client))
    }

    var body: some Scene {
        WindowGroup("Panops") {
            ContentView(vm: viewModel)
                .task { await bootstrap() }
                .alert("Engine failed to start", isPresented: errorPresented) {
                    Button("Quit", role: .destructive) { NSApp.terminate(nil) }
                } message: {
                    Text(startupError ?? "")
                }
        }
        .windowResizability(.contentSize)
    }

    private var errorPresented: Binding<Bool> {
        Binding(
            get: { startupError != nil },
            set: { if !$0 { startupError = nil } }
        )
    }

    private func bootstrap() async {
        do {
            engine = try EngineProcess.start()
        } catch EngineProcess.LookupError.binaryNotFound(let env, let bundle) {
            startupError = """
            Could not find panops-engine binary.
            Tried PANOPS_ENGINE_BIN=\(env ?? "(unset)") and bundle path \(bundle ?? "(unknown)").
            Set PANOPS_ENGINE_BIN to the absolute path (dev) or rebuild the .app (prod).
            """
            return
        } catch {
            startupError = "\(error)"
            return
        }
        do {
            try await viewModel.connect()
        } catch {
            startupError = "IPC connect failed: \(error)"
            return
        }
        NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification,
            object: nil,
            queue: .main
        ) { [engineCopy = engine] _ in
            // SIGTERM the engine synchronously. We cannot `await
            // viewModel.shutdown(...)` here: AppViewModel is `@MainActor`
            // and blocking main with a DispatchSemaphore to wait on the
            // async hop deadlocks. The engine being SIGTERM'd is the
            // important part — the IPC connection closes on socket
            // teardown anyway, and the OS reaps the child process when
            // the app exits.
            engineCopy?.terminateSync()
        }
    }
}
