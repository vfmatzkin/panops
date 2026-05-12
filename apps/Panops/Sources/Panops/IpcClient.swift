import Foundation
import Network

/// Errors surfaced to UI code; details intentionally short. Full
/// detail goes to engine stderr (Console.app stream).
enum IpcClientError: Error {
    case socketUnavailable(String)
    case rpcError(code: Int, message: String)
    case decode(String)
    case disconnected
}

/// JSON-RPC + WebSocket client for the panops engine UDS.
/// Task 4 implements the JSON-RPC half. Task 5 adds event subscription.
actor IpcClient {
    private let endpoint: NWEndpoint
    private var rpcConnection: NWConnection?
    private var nextId: UInt64 = 1

    init(socketPath: URL) {
        // socketPath is the resolved absolute path to engine.sock
        self.endpoint = .unix(path: socketPath.path)
    }

    /// Open the JSON-RPC connection. Retries with exponential backoff
    /// up to 5 seconds total to absorb engine cold-start.
    func connect() async throws {
        let deadline = Date().addingTimeInterval(5)
        var delayMs: UInt64 = 100
        while Date() < deadline {
            do {
                let conn = NWConnection(to: endpoint, using: .tcp)
                try await Self.start(conn)
                self.rpcConnection = conn
                return
            } catch {
                try? await Task.sleep(nanoseconds: delayMs * 1_000_000)
                delayMs = min(delayMs * 2, 1_500)
            }
        }
        throw IpcClientError.socketUnavailable(
            "engine.sock not ready within 5s"
        )
    }

    /// Disconnect and release resources.
    func disconnect() async {
        rpcConnection?.cancel()
        rpcConnection = nil
    }

    /// `ipc.notes.generate` — returns the job_id; the JobDone result
    /// arrives later via the WebSocket events (Task 5).
    func notesGenerate(
        audio: URL,
        dialect: String? = nil,
        language: String? = nil,
        meetingId: String? = nil
    ) async throws -> String {
        let params = NotesGenerateParams(
            audio: audio.path,
            dialect: dialect,
            language: language,
            llmProvider: nil,
            llmModel: nil,
            noDiarize: nil,
            meetingId: meetingId
        )
        let result: NotesGenerateResult = try await sendRequest(
            method: "ipc.notes.generate",
            params: params
        )
        return result.jobId
    }

    // MARK: - Private

    private func sendRequest<P: Encodable, R: Decodable>(
        method: String,
        params: P
    ) async throws -> R {
        guard let conn = rpcConnection else {
            throw IpcClientError.disconnected
        }
        let id = nextId
        nextId += 1
        let envelope = JsonRpcRequest(id: id, method: method, params: params)
        let encoder = JSONEncoder()
        let body = try encoder.encode(envelope)
        // Newline-delimited framing — matches jsonrpsee's stdio mode.
        // If smoke shows the engine expects HTTP framing, revisit here.
        var framed = body
        framed.append(0x0A)
        try await Self.send(conn, data: framed)
        let respData = try await Self.receiveLine(conn)
        let decoder = JSONDecoder()
        let resp = try decoder.decode(JsonRpcResponse<R>.self, from: respData)
        if let err = resp.error {
            throw IpcClientError.rpcError(code: err.code, message: err.message)
        }
        guard let result = resp.result else {
            throw IpcClientError.decode("response missing both result and error")
        }
        return result
    }

    private static func start(_ conn: NWConnection) async throws {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
            conn.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    cont.resume(returning: ())
                case .failed(let err):
                    cont.resume(throwing: err)
                case .cancelled:
                    cont.resume(throwing: IpcClientError.disconnected)
                default:
                    break
                }
            }
            conn.start(queue: .global())
        }
    }

    private static func send(_ conn: NWConnection, data: Data) async throws {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
            conn.send(content: data, completion: .contentProcessed { error in
                if let error = error {
                    cont.resume(throwing: error)
                } else {
                    cont.resume(returning: ())
                }
            })
        }
    }

    private static func receiveLine(_ conn: NWConnection) async throws -> Data {
        var buffer = Data()
        while true {
            let chunk: Data = try await withCheckedThrowingContinuation { cont in
                conn.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { data, _, isComplete, error in
                    if let error = error {
                        cont.resume(throwing: error)
                    } else if let data = data, !data.isEmpty {
                        cont.resume(returning: data)
                    } else if isComplete {
                        cont.resume(throwing: IpcClientError.disconnected)
                    } else {
                        cont.resume(returning: Data())
                    }
                }
            }
            buffer.append(chunk)
            if let lf = buffer.firstIndex(of: 0x0A) {
                let line = buffer.subdata(in: buffer.startIndex..<lf)
                return line
            }
        }
    }
}
