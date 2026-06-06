import Foundation
import Network

/// Errors surfaced to UI code; details intentionally short. Full
/// detail goes to engine stderr (Console.app stream).
enum IpcClientError: Error {
    case socketUnavailable(String)
    case rpcError(code: Int, message: String)
    case decode(String)
    case disconnected
    case httpError(status: Int, body: String)
    case websocketUpgradeFailed(String)
    case websocketFrameError(String)
}

/// Minimal WebSocket frame parser per RFC 6455.
/// Handles text frames (opcode 0x01) with FIN=1 only.
/// Unmaskes client-to-server frames, decodes payload as JSON.
/// Slice 12 implementation per spec D6.
struct WsFrameParser {
    /// Parse a single WebSocket frame from raw bytes.
    /// Returns decoded JSON data for text frames, nil for non-text.
    static func parse(_ data: Data) throws -> Data? {
        guard data.count >= 2 else {
            throw IpcClientError.websocketFrameError("frame too short")
        }

        let fin = (data[0] & 0x80) != 0
        let opcode = data[0] & 0x0F
        let masked = (data[1] & 0x80) != 0
        var payloadLen = Int(data[1] & 0x7F)

        // Only handle text frames (opcode 0x01)
        guard opcode == 0x01 else {
            return nil // ignore binary/ping/close
        }

        // Only handle FIN=1 frames (no fragmentation)
        guard fin else {
            return nil // ignore fragmented frames
        }

        // Calculate header offset based on payload length encoding
        var offset = 2
        if payloadLen == 126 {
            guard data.count >= 4 else {
                throw IpcClientError.websocketFrameError("extended length missing")
            }
            payloadLen = Int(data[2]) << 8 | Int(data[3])
            offset = 4
        } else if payloadLen == 127 {
            guard data.count >= 10 else {
                throw IpcClientError.websocketFrameError("extended length missing")
            }
            // For our use case, we only need 32-bit length
            payloadLen = Int(data[6]) << 24 | Int(data[7]) << 16 | Int(data[8]) << 8 | Int(data[9])
            offset = 10
        }

        // Check for masking key if masked
        let maskingKeyOffset = offset
        if masked {
            guard data.count >= offset + 4 + payloadLen else {
                throw IpcClientError.websocketFrameError("frame truncated")
            }
            offset += 4
        } else {
            guard data.count >= offset + payloadLen else {
                throw IpcClientError.websocketFrameError("frame truncated")
            }
        }

        // Extract and unmask payload
        let payloadStart = offset
        let payloadData = data.subdata(in: payloadStart..<payloadStart + payloadLen)

        if masked {
            let mask = data.subdata(in: maskingKeyOffset..<maskingKeyOffset + 4)
            var unmasked = Data(count: payloadLen)
            for i in 0..<payloadLen {
                unmasked[i] = payloadData[i] ^ mask[i % 4]
            }
            return unmasked
        } else {
            return payloadData
        }
    }
}

/// JSON-RPC client for the panops engine UDS.
///
/// Transport: each request is a fresh HTTP POST over a one-shot
/// `NWConnection` to the UDS. Walking-skeleton choice per slice-09
/// design decision D3 (2026-05-12). The engine's jsonrpsee server
/// accepts POST `/` with `application/json` body and replies with
/// `200 OK` + JSON-RPC envelope.
///
/// Slice 09 does NOT implement WebSocket subscription. Job-completion
/// detection happens via polling `meeting.get(id)` in the view model.
/// Proper event-driven UX (hand-rolled RFC 6455 WS) is a follow-up
/// slice. The spike at Task 3 proved the engine accepts manual WS
/// upgrade over UDS; `NWProtocolWebSocket.Options` does not.
actor IpcClient {
    private let endpoint: NWEndpoint
    private let socketPath: URL
    private var nextId: UInt64 = 1
    private var wsConnection: NWConnection?

    init(socketPath: URL) {
        self.socketPath = socketPath
        self.endpoint = .unix(path: socketPath.path)
    }

    /// Probe the socket is bindable. Returns when the engine accepts
    /// one connection (5s deadline with exponential backoff).
    /// Cancellation-aware: if the caller cancels (e.g. app quits while
    /// the engine is slow to start), exits promptly via
    /// `Task.checkCancellation()`.
    func connect() async throws {
        let deadline = Date().addingTimeInterval(5)
        var delayMs: UInt64 = 100
        while Date() < deadline {
            try Task.checkCancellation()
            let conn = NWConnection(to: endpoint, using: .tcp)
            defer { conn.cancel() }
            do {
                try await Self.start(conn)
                return
            } catch {
                try? await Task.sleep(nanoseconds: delayMs * 1_000_000)
                try Task.checkCancellation()
                delayMs = min(delayMs * 2, 1_500)
            }
        }
        throw IpcClientError.socketUnavailable(
            "engine.sock not ready within 5s at \(socketPath.path)"
        )
    }

    /// No-op for HTTP-POST transport (no persistent connection).
    /// Kept on the API for symmetry with future WS-based transport.
    func disconnect() async {
        wsConnection?.cancel()
        wsConnection = nil
    }

    /// WebSocket upgrade: hand-rolled RFC 6455 over UDS.
    /// Sends HTTP GET with Upgrade headers, expects 101 response.
    /// Stores the connection for subsequent frame reads.
    /// Slice 12 implementation per spec D6.
    func wsConnect() async throws {
        // Generate WebSocket key (16 bytes base64-encoded)
        // Use a deterministic key for testing; real impl would use random
        let keyData = Data([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15])
        let wsKey = keyData.base64EncodedString()

        let request = Self.buildWsUpgradeRequest(key: wsKey)
        let conn = NWConnection(to: endpoint, using: .tcp)
        try await Self.start(conn)

        try await Self.send(conn, data: request)
        let status = try await Self.readWsUpgradeResponse(conn)

        guard status == 101 else {
            conn.cancel()
            throw IpcClientError.websocketUpgradeFailed(
                "expected HTTP 101, got \(status)"
            )
        }

        // Connection stays open for WebSocket frames
        wsConnection = conn
    }

    /// Subscribe to IPC events via WebSocket.
    /// Sends `ipc.events.subscribe` and returns an AsyncStream of decoded events.
    /// Requires wsConnect() to have been called first.
    /// Slice 12 implementation per spec D6.
    func subscribeEvents() async throws -> AsyncStream<IpcEvent> {
        guard let conn = wsConnection else {
            throw IpcClientError.websocketUpgradeFailed(
                "wsConnect() must be called before subscribeEvents()"
            )
        }

        // Send ipc.events.subscribe as a WebSocket text frame
        let id = nextId
        nextId += 1
        let subscribeRequest = JsonRpcRequest(id: id, method: "ipc.events.subscribe", param: EmptyParams())
        let body = try JSONEncoder().encode(subscribeRequest)
        let frame = Self.buildWsTextFrame(body)
        try await Self.send(conn, data: frame)

        // Create AsyncStream that yields decoded events
        return AsyncStream { continuation in
            Task {
                var buffer = Data()
                while !Task.isCancelled {
                    do {
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

                        // Try to parse frames from buffer
                        // parseFrameFromBuffer returns Data? - nil for non-text or incomplete
                        let parseResult = try Self.parseFrameFromBuffer(&buffer)
                        if let eventData = parseResult {
                            let event = try JSONDecoder().decode(IpcEvent.self, from: eventData)
                            continuation.yield(event)
                        }
                    } catch {
                        continuation.finish()
                        break
                    }
                }
                continuation.finish()
            }
        }
    }

    /// Build a WebSocket text frame with masked payload (client-to-server).
    private static func buildWsTextFrame(_ payload: Data) -> Data {
        // Client frames must be masked per RFC 6455
        let maskKey: [UInt8] = [0x01, 0x02, 0x03, 0x04] // deterministic for testing
        var frame = Data()

        // FIN=1, opcode=0x01 (text)
        frame.append(0x81)

        // Masked bit + length
        let len = payload.count
        if len <= 125 {
            frame.append(0x80 | UInt8(len))
        } else if len <= 65535 {
            frame.append(0x80 | 126)
            frame.append(UInt8(len >> 8))
            frame.append(UInt8(len & 0xFF))
        } else {
            frame.append(0x80 | 127)
            // 8-byte length (we only need 4 bytes for reasonable payloads)
            frame.append(contentsOf: [0, 0, 0, 0])
            frame.append(UInt8(len >> 24))
            frame.append(UInt8((len >> 16) & 0xFF))
            frame.append(UInt8((len >> 8) & 0xFF))
            frame.append(UInt8(len & 0xFF))
        }

        // Masking key
        frame.append(contentsOf: maskKey)

        // Masked payload
        for i in 0..<payload.count {
            frame.append(payload[i] ^ maskKey[i % 4])
        }

        return frame
    }

    /// Parse a single frame from the buffer, removing consumed bytes.
    /// Returns the payload data for text frames, nil for non-text or incomplete.
    private static func parseFrameFromBuffer(_ buffer: inout Data) throws -> Data? {
        guard buffer.count >= 2 else { return nil }

        let fin = (buffer[0] & 0x80) != 0
        let opcode = buffer[0] & 0x0F
        var payloadLen = Int(buffer[1] & 0x7F)

        // Only handle text frames
        guard opcode == 0x01 else {
            // Skip non-text frames by parsing length and removing from buffer
            let frameLen = Self.calculateFrameLength(buffer)
            if frameLen > 0 && buffer.count >= frameLen {
                buffer = Data(buffer.dropFirst(frameLen))
                return nil // Continue parsing
            }
            return nil // Need more data
        }

        // Only handle FIN=1 frames
        guard fin else { return nil }

        // Calculate header size
        var headerSize = 2
        if payloadLen == 126 {
            guard buffer.count >= 4 else { return nil }
            payloadLen = Int(buffer[2]) << 8 | Int(buffer[3])
            headerSize = 4
        } else if payloadLen == 127 {
            guard buffer.count >= 10 else { return nil }
            payloadLen = Int(buffer[6]) << 24 | Int(buffer[7]) << 16 | Int(buffer[8]) << 8 | Int(buffer[9])
            headerSize = 10
        }

        // Server frames are unmasked
        let totalLen = headerSize + payloadLen
        guard buffer.count >= totalLen else { return nil }

        // Extract payload
        let payload = Data(buffer.subdata(in: headerSize..<totalLen))
        buffer = Data(buffer.dropFirst(totalLen))
        return payload
    }

    /// Calculate total frame length for any frame type.
    private static func calculateFrameLength(_ buffer: Data) -> Int {
        guard buffer.count >= 2 else { return 0 }

        var payloadLen = Int(buffer[1] & 0x7F)
        var headerSize = 2

        if payloadLen == 126 {
            guard buffer.count >= 4 else { return 0 }
            payloadLen = Int(buffer[2]) << 8 | Int(buffer[3])
            headerSize = 4
        } else if payloadLen == 127 {
            guard buffer.count >= 10 else { return 0 }
            payloadLen = Int(buffer[6]) << 24 | Int(buffer[7]) << 16 | Int(buffer[8]) << 8 | Int(buffer[9])
            headerSize = 10
        }

        let masked = (buffer[1] & 0x80) != 0
        if masked {
            headerSize += 4 // masking key
        }

        return headerSize + payloadLen
    }

    private static func buildWsUpgradeRequest(key: String) -> Data {
        let header = """
        GET / HTTP/1.1\r
        Host: localhost\r
        Upgrade: websocket\r
        Connection: Upgrade\r
        Sec-WebSocket-Key: \(key)\r
        Sec-WebSocket-Version: 13\r
        \r

        """
        return Data(header.utf8)
    }

    private static func readWsUpgradeResponse(_ conn: NWConnection) async throws -> Int {
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
            if buffer.count > Self.maxHeaderBytes {
                throw IpcClientError.decode(
                    "HTTP header exceeded \(Self.maxHeaderBytes) bytes"
                )
            }
            if let sepRange = buffer.range(of: Data("\r\n\r\n".utf8)) {
                let headerData = buffer.subdata(in: buffer.startIndex..<sepRange.lowerBound)
                guard let headerText = String(data: headerData, encoding: .utf8) else {
                    throw IpcClientError.decode("non-utf8 HTTP header")
                }
                let lines = headerText.components(separatedBy: "\r\n")
                guard let statusLine = lines.first else {
                    throw IpcClientError.decode("missing HTTP status line")
                }
                let parts = statusLine.split(separator: " ", maxSplits: 2)
                guard parts.count >= 2, let status = Int(parts[1]) else {
                    throw IpcClientError.decode("invalid HTTP status line: \(statusLine)")
                }
                return status
            }
        }
    }

    /// `ipc.notes.generate` — synchronous response is the job_id.
    /// JobDone arrives later; slice 09 detects completion via polling
    /// `meeting.get(meetingId)` rather than WS events.
    func notesGenerate(
        audio: URL,
        meetingId: String,
        dialect: String? = nil,
        language: String? = nil
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

    /// `ipc.meeting.start` — auto-creates a meeting row and returns
    /// the meeting_id (a bare JSON string). Slice-06 allows this
    /// without live capture; the returned id is used for the polling
    /// loop.
    func meetingStart() async throws -> String {
        let result: String = try await sendRequest(
            method: "ipc.meeting.start",
            params: EmptyParams()
        )
        return result
    }

    /// `ipc.meeting.get(id)` — fetches the Meeting row. The returned
    /// `dirPath` is where `notes.generate` will write its output.
    /// Slice 09 polls the filesystem under `dirPath/notes.md` rather
    /// than calling `meeting.get` repeatedly — the IPC's `Meeting`
    /// type doesn't carry note-completion metadata.
    func meetingGet(id: String) async throws -> Meeting {
        return try await sendRequest(
            method: "ipc.meeting.get",
            params: MeetingGetParams(id: id)
        )
    }

    /// `ipc.meeting.list` — fetches all meeting summaries.
    /// Slice 12: sidebar needs meeting list for navigation.
    func meetingList() async throws -> [MeetingSummary] {
        return try await sendRequest(
            method: "ipc.meeting.list",
            params: EmptyParams()
        )
    }

    // MARK: - Private

    private func sendRequest<P: Encodable, R: Decodable>(
        method: String,
        params: P
    ) async throws -> R {
        let id = nextId
        nextId += 1
        let envelope = JsonRpcRequest(id: id, method: method, param: params)
        let body = try JSONEncoder().encode(envelope)
        let request = Self.buildHttpRequest(body: body)
        let conn = NWConnection(to: endpoint, using: .tcp)
        try await Self.start(conn)
        defer { conn.cancel() }
        try await Self.send(conn, data: request)
        let (status, responseBody) = try await Self.readHttpResponse(conn)
        guard status == 200 else {
            let bodyString = String(data: responseBody, encoding: .utf8) ?? ""
            throw IpcClientError.httpError(status: status, body: bodyString)
        }
        let resp = try JSONDecoder().decode(JsonRpcResponse<R>.self, from: responseBody)
        if let err = resp.error {
            throw IpcClientError.rpcError(code: err.code, message: err.message)
        }
        guard let result = resp.result else {
            throw IpcClientError.decode("response missing both result and error")
        }
        return result
    }

    private static func buildHttpRequest(body: Data) -> Data {
        let header = """
        POST / HTTP/1.1\r
        Host: localhost\r
        Content-Type: application/json\r
        Content-Length: \(body.count)\r
        Connection: close\r
        \r

        """
        var req = Data(header.utf8)
        req.append(body)
        return req
    }

    /// Maximum bytes the HTTP header section is allowed to occupy
    /// before we give up and treat the response as malformed. The
    /// engine's responses are tiny (typically <300 bytes including
    /// JSON body); 8 KB is several orders of magnitude of safety.
    private static let maxHeaderBytes = 8 * 1024

    private static func readHttpResponse(_ conn: NWConnection) async throws -> (status: Int, body: Data) {
        // Read until \r\n\r\n to separate header from body, then read
        // body length per Content-Length. Engine sets Connection: close
        // so we could also read until EOF, but Content-Length is cleaner.
        var buffer = Data()
        while true {
            let chunk: Data = try await withCheckedThrowingContinuation { cont in
                conn.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { data, _, isComplete, error in
                    if let error = error {
                        cont.resume(throwing: error)
                    } else if let data = data, !data.isEmpty {
                        cont.resume(returning: data)
                    } else if isComplete {
                        // EOF before headers complete — error.
                        cont.resume(throwing: IpcClientError.disconnected)
                    } else {
                        cont.resume(returning: Data())
                    }
                }
            }
            buffer.append(chunk)
            if buffer.count > Self.maxHeaderBytes,
                buffer.range(of: Data("\r\n\r\n".utf8)) == nil {
                throw IpcClientError.decode(
                    "HTTP header exceeded \(Self.maxHeaderBytes) bytes without separator"
                )
            }
            if let sepRange = buffer.range(of: Data("\r\n\r\n".utf8)) {
                let headerData = buffer.subdata(in: buffer.startIndex..<sepRange.lowerBound)
                let bodyStart = sepRange.upperBound
                guard let headerText = String(data: headerData, encoding: .utf8) else {
                    throw IpcClientError.decode("non-utf8 HTTP header")
                }
                // Split on CRLF using `components(separatedBy:)` (the
                // Swift String API accepts a multi-character separator
                // there, unlike the `Collection.split` element variant).
                let lines = headerText.components(separatedBy: "\r\n")
                guard let statusLine = lines.first else {
                    throw IpcClientError.decode("missing HTTP status line")
                }
                let parts = statusLine.split(separator: " ", maxSplits: 2)
                guard parts.count >= 2, let status = Int(parts[1]) else {
                    throw IpcClientError.decode("invalid HTTP status line: \(statusLine)")
                }
                // Find Content-Length.
                var contentLength: Int = 0
                for line in lines.dropFirst() {
                    let lower = line.lowercased()
                    if lower.hasPrefix("content-length:") {
                        let value = line.dropFirst("content-length:".count).trimmingCharacters(in: .whitespaces)
                        contentLength = Int(value) ?? 0
                    }
                }
                var body = buffer.subdata(in: bodyStart..<buffer.endIndex)
                while body.count < contentLength {
                    let more: Data = try await withCheckedThrowingContinuation { cont in
                        conn.receive(minimumIncompleteLength: 1, maximumLength: contentLength - body.count) { data, _, isComplete, error in
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
                    body.append(more)
                }
                return (status, body)
            }
        }
    }

    private static func start(_ conn: NWConnection) async throws {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
            final class ResumedGuard: @unchecked Sendable {
                var value = false
            }
            let resumed = ResumedGuard()
            conn.stateUpdateHandler = { state in
                guard !resumed.value else { return }
                switch state {
                case .ready:
                    resumed.value = true
                    cont.resume(returning: ())
                case .failed(let err):
                    resumed.value = true
                    cont.resume(throwing: err)
                case .cancelled:
                    resumed.value = true
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
}
