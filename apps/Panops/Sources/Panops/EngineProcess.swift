import Foundation

/// Spawns the panops-engine binary and owns its lifecycle.
/// The Mac shell quitting must result in a clean engine shutdown.
struct EngineProcess {
    enum LookupError: Error {
        case binaryNotFound(triedEnv: String?, triedBundle: String?)
        case launchFailed(String)
    }

    let process: Process

    /// Resolve the engine binary, spawn it, return a handle.
    /// Resolution order:
    /// 1. `PANOPS_ENGINE_BIN` env var (dev escape hatch, spec D3).
    /// 2. `Bundle.main.bundleURL/Contents/Resources/panops-engine` (production).
    static func start() throws -> EngineProcess {
        let envPath = ProcessInfo.processInfo.environment["PANOPS_ENGINE_BIN"]
        let bundlePath = Bundle.main.bundleURL
            .appendingPathComponent("Contents/Resources/panops-engine")
            .path
        let candidates: [String?] = [envPath, bundlePath]
        let resolved = candidates.compactMap { $0 }.first { path in
            FileManager.default.isExecutableFile(atPath: path)
        }
        guard let binary = resolved else {
            throw LookupError.binaryNotFound(
                triedEnv: envPath, triedBundle: bundlePath
            )
        }

        let p = Process()
        p.executableURL = URL(fileURLWithPath: binary)
        p.arguments = ["serve"]
        // Pipe engine stderr to the host stderr so logs surface in
        // Console.app.
        p.standardError = FileHandle.standardError
        // Engine stdout intentionally silenced for the shell — engine
        // emits JSON only via the socket, not stdout.
        p.standardOutput = Pipe()
        do {
            try p.run()
        } catch {
            throw LookupError.launchFailed("\(error)")
        }
        return EngineProcess(process: p)
    }

    var isRunning: Bool { process.isRunning }

    /// Send SIGTERM and wait up to 5s; then SIGKILL if still alive.
    func stop() async {
        guard process.isRunning else { return }
        process.terminate()  // SIGTERM
        let deadline = Date().addingTimeInterval(5)
        while process.isRunning && Date() < deadline {
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        if process.isRunning {
            kill(process.processIdentifier, SIGKILL)
            process.waitUntilExit()
        }
    }
}
