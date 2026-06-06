import Foundation
import Network
import CommonCrypto

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
    case websocketAcceptMismatch
}

/// Minimal WebSocket frame parser per RFC 6455.
/// Handles text frames (opcode 0x01) and continuation frames (opcode 0x00).
/// Server→client frames are unmasked; client→server frames must be masked.
/// Slice 12 implementation per spec D6.
struct WsFrameParser {
    /// Frame metadata extracted from the first 2 bytes.
    struct FrameHeader {
        let fin: Bool
        let opcode: UInt8
        let masked: Bool
        let payloadLen: Int
        let headerSize: Int
        let maskingKeyOffset: Int?
    }

    /// Parse frame header from data. Returns nil if data is incomplete.
    static func parseHeader(_ data: Data) -> FrameHeader? {
        guard data.count >= 2 else { return nil }

        let fin = (data[0] & 0x80) != 0
        let opcode = data[0] & 0x0F
        let masked = (data[1] & 0x80) != 0
        var payloadLen = Int(data[1] & 0x7F)

        var headerSize = 2
        if payloadLen == 126 {
            guard data.count >= 4 else { return nil }
            payloadLen = Int(data[2]) << 8 | Int(data[3])
            headerSize = 4
        } else if payloadLen == 127 {
            guard data.count >= 10 else { return nil }
            payloadLen = Int(data[6]) << 24 | Int(data[7]) << 16 | Int(data[8]) << 8 | Int(data[9])
            headerSize = 10
        }

        let maskingKeyOffset = masked ? headerSize : nil
        if masked {
            headerSize += 4
        }

        return FrameHeader(
            fin: fin,
            opcode: opcode,
            masked: masked,
            payloadLen: payloadLen,
            headerSize: headerSize,
            maskingKeyOffset: maskingKeyOffset
        )
    }

    /// Calculate total frame length including header + masking key + payload.
    static func totalFrameLength(_ header: FrameHeader) -> Int {
        return header.headerSize + header.payloadLen
    }

    /// Extract payload from frame data given header.
    /// Returns nil if data is incomplete.
    static func extractPayload(_ data: Data, header: FrameHeader) -> Data? {
        let totalLen = totalFrameLength(header)
        guard data.count >= totalLen else { return nil }

        let payloadStart = header.headerSize
        let payloadEnd = payloadStart + header.payloadLen
        let payloadData = data.subdata(in: payloadStart..<payloadEnd)

        if let maskOffset = header.maskingKeyOffset {
            let mask = data.subdata(in: maskOffset..<maskOffset + 4)
            var unmasked = Data(count: header.payloadLen)
            for i in 0..<header.payloadLen {
                unmasked[i] = payloadData[i] ^ mask[i % 4]
            }
            return unmasked
        } else {
            return payloadData
        }
    }

    /// Parse a complete WebSocket frame from raw bytes.
    /// Returns (opcode, payload) for text/continuation frames, nil for non-text/incomplete.
    static func parse(_ data: Data) throws -> Data? {
        guard let header = parseHeader(data) else {
            guard data.count >= 2 else {
                throw IpcClientError.websocketFrameError("frame too short")
            }
            return nil // incomplete frame
        }

        // Only handle text frames (opcode 0x01) and continuation (0x00)
        guard header.opcode == 0x01 || header.opcode == 0x00 else {
            return nil // ignore binary/ping/close
        }

        // Only handle FIN=1 frames for text (no fragmentation for text)
        // Continuation frames (opcode 0x00) can have FIN=0 until the final FIN=1
        if header.opcode == 0x01 && !header.fin {
            return nil // ignore fragmented text start frames
        }

        guard let payload = extractPayload(data, header: header) else {
            return nil // incomplete
        }

        return payload
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
    /// Verifies Sec-WebSocket-Accept per RFC 6455 §4.2.2.
    /// Stores the connection for subsequent frame reads.
    /// Slice 12 implementation per spec D6.
    func wsConnect() async throws {
        // Generate random 16-byte WebSocket key per RFC 6455
        var keyData = Data(count: 16)
        let keyResult = keyData.withUnsafeMutableBytes { ptr in
            SecRandomCopyBytes(kSecRandomDefault, 16, ptr.baseAddress!)
        }
        guard keyResult == errSecSuccess else {
            throw IpcClientError.websocketUpgradeFailed("failed to generate random WebSocket key")
        }
        let wsKey = keyData.base64EncodedString()

        let request = Self.buildWsUpgradeRequest(key: wsKey)
        let conn = NWConnection(to: endpoint, using: .tcp)
        try await Self.start(conn)

        // Use defer to cancel connection on any early exit (NWConnection leak fix)
        var connectionStored = false
        defer {
            if !connectionStored {
                conn.cancel()
            }
        }

        try await Self.send(conn, data: request)
        let (status, acceptHeader) = try await Self.readWsUpgradeResponse(conn, expectedKey: wsKey)

        guard status == 101 else {
            throw IpcClientError.websocketUpgradeFailed(
                "expected HTTP 101, got \(status)"
            )
        }

        // Verify Sec-WebSocket-Accept per RFC 6455 §4.2.2
        // accept = base64(SHA1(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"))
        guard let acceptHeader else {
            throw IpcClientError.websocketUpgradeFailed("missing Sec-WebSocket-Accept header")
        }
        let expectedAccept = Self.computeWebSocketAccept(key: wsKey)
        guard acceptHeader == expectedAccept else {
            throw IpcClientError.websocketAcceptMismatch
        }

        // Connection stays open for WebSocket frames
        wsConnection = conn
        connectionStored = true
    }

    /// Compute Sec-WebSocket-Accept per RFC 6455 §4.2.2.
    /// accept = base64(SHA1(key + GUID))
    private static func computeWebSocketAccept(key: String) -> String {
        let guid = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
        let combined = key + guid
        let combinedData = Data(combined.utf8)

        // SHA1 hash
        var hash = Data(count: Int(CC_SHA1_DIGEST_LENGTH))
        combinedData.withUnsafeBytes { combinedPtr in
            hash.withUnsafeMutableBytes { hashPtr in
                _ = CC_SHA1(combinedPtr.baseAddress!, CC_LONG(combinedData.count), hashPtr.baseAddress!)
            }
        }

        return hash.base64EncodedString()
    }

    /// Subscribe to IPC events via WebSocket.
    /// Wire flow: client sends events.subscribe (no params) → server replies with subscriptionId
    /// → events arrive as notifications {method:"events", params:{subscription, result}}
    /// The Event payload is params.result, NOT the whole frame.
    /// Slice 12 implementation per spec D6.
    func subscribeEvents() async throws -> AsyncStream<IpcEvent> {
        guard let conn = wsConnection else {
            throw IpcClientError.websocketUpgradeFailed(
                "wsConnect() must be called before subscribeEvents()"
            )
        }

        // Send ipc.events.subscribe with no params (namespace prefix per jsonrpsee)
        let id = nextId
        nextId += 1
        let subscribeRequest = JsonRpcRequestNoParams(id: id, method: "ipc.events.subscribe")
        let body = try JSONEncoder().encode(subscribeRequest)
        let frame = Self.buildWsTextFrame(body)
        try await Self.send(conn, data: frame)

        // Read the subscription-id result reply (first frame after subscribe)
        var replyBuffer = Data()
        let replyData = try await Self.readWsFrame(conn, buffer: &replyBuffer)
        guard let replyFrameData = replyData else {
            throw IpcClientError.websocketFrameError("no reply for events.subscribe")
        }
        let reply = try JSONDecoder().decode(JsonRpcResponse<SubscriptionId>.self, from: replyFrameData)
        guard reply.id == id else {
            throw IpcClientError.websocketFrameError("wrong id in events.subscribe reply")
        }
        if let err = reply.error {
            throw IpcClientError.rpcError(code: err.code, message: err.message)
        }
        guard reply.result != nil else {
            throw IpcClientError.websocketFrameError("events.subscribe reply missing result")
        }
        // The subscriptionId in reply.result is echoed in each notification's
        // params.subscription field; we don't need to track it directly.

        // Create AsyncStream that yields decoded events from notifications
        // Copy buffer data before entering the closure to avoid data race
        let initialBufferData = Data(replyBuffer)
        return AsyncStream { continuation in
            Task {
                var buffer = initialBufferData  // continue with any remaining data
                while !Task.isCancelled {
                    do {
                        let frameData = try await Self.readWsFrame(conn, buffer: &buffer)
                        guard let eventData = frameData else {
                            // Non-text frame or incomplete, continue
                            continue
                        }
                        // Decode notification envelope and extract params.result
                        let notification = try JSONDecoder().decode(JsonRpcNotification.self, from: eventData)
                        // The Event payload is in params.result
                        continuation.yield(notification.params.result)
                    } catch {
                        continuation.finish()
                        break
                    }
                }
                continuation.finish()
            }
        }
    }

    /// Read a single WebSocket frame from the connection.
    /// Returns payload Data for text frames (opcode 0x01), nil for non-text.
    /// Handles fragmented frames: FIN=0 frames are consumed and accumulated
    /// until FIN=1 arrives, then the reassembled payload is returned.
    private static func readWsFrame(_ conn: NWConnection, buffer: inout Data) async throws -> Data? {
        // Accumulate fragmented frame payloads
        var accumulatedPayload = Data()
        var waitingForContinuation = false

        while true {
            // Read more data if buffer doesn't have a complete frame header
            while buffer.count < 2 {
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
            }

            // Parse frame header
            guard let header = WsFrameParser.parseHeader(buffer) else {
                // Incomplete header - need more data
                continue
            }

            // Check if we have the complete frame
            let totalLen = WsFrameParser.totalFrameLength(header)
            guard buffer.count >= totalLen else {
                // Incomplete frame - need more data
                continue
            }

            // Check opcode
            if header.opcode == 0x01 { // Text frame
                // If we're not waiting for continuation, this is a new message
                // If we are waiting, this is invalid (text in middle of continuation)
                guard !waitingForContinuation else {
                    // Invalid: received text frame while waiting for continuation
                    buffer = Data(buffer.dropFirst(totalLen))
                    continue
                }

                // Extract payload
                guard let payload = WsFrameParser.extractPayload(buffer, header: header) else {
                    buffer = Data(buffer.dropFirst(totalLen))
                    continue
                }

                // Consume frame from buffer
                buffer = Data(buffer.dropFirst(totalLen))

                if header.fin {
                    // Complete text frame - return payload
                    return payload
                } else {
                    // Start of fragmented message - accumulate and wait for continuation
                    accumulatedPayload = payload
                    waitingForContinuation = true
                    continue
                }
            } else if header.opcode == 0x00 { // Continuation frame
                // Must be in continuation mode
                guard waitingForContinuation else {
                    // Invalid: received continuation without start frame
                    buffer = Data(buffer.dropFirst(totalLen))
                    continue
                }

                // Extract payload
                guard let payload = WsFrameParser.extractPayload(buffer, header: header) else {
                    buffer = Data(buffer.dropFirst(totalLen))
                    continue
                }

                // Consume frame from buffer
                buffer = Data(buffer.dropFirst(totalLen))

                // Append to accumulated payload
                accumulatedPayload.append(payload)

                if header.fin {
                    // Final continuation frame - return reassembled payload
                    waitingForContinuation = false
                    return accumulatedPayload
                } else {
                    // More continuation frames expected
                    continue
                }
            } else {
                // Non-text frame (binary, ping, close, etc.) - consume and ignore
                buffer = Data(buffer.dropFirst(totalLen))
                if waitingForContinuation {
                    // Invalid: non-text frame during continuation - abort
                    waitingForContinuation = false
                    accumulatedPayload = Data()
                }
                continue
            }
        }
    }

    /// Build a WebSocket text frame with masked payload (client-to-server).
    /// Per RFC 6455, client frames MUST use a random mask per frame.
    private static func buildWsTextFrame(_ payload: Data) -> Data {
        // Generate random 4-byte mask per RFC 6455
        var maskKey = Data(count: 4)
        maskKey.withUnsafeMutableBytes { ptr in
            _ = SecRandomCopyBytes(kSecRandomDefault, 4, ptr.baseAddress!)
        }

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
        frame.append(maskKey)

        // Masked payload
        for i in 0..<payload.count {
            frame.append(payload[i] ^ maskKey[i % 4])
        }

        return frame
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

    private static func readWsUpgradeResponse(_ conn: NWConnection, expectedKey: String) async throws -> (status: Int, accept: String?) {
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
                // Extract Sec-WebSocket-Accept header
                var acceptHeader: String?
                for line in lines.dropFirst() {
                    let lower = line.lowercased()
                    if lower.hasPrefix("sec-websocket-accept:") {
                        let value = line.dropFirst("sec-websocket-accept:".count).trimmingCharacters(in: .whitespaces)
                        acceptHeader = value
                    }
                }
                return (status, acceptHeader)
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
        // meeting.start takes a MeetingConfig param (optional title/language);
        // it is NOT a no-param method. EmptyParams() encodes to {} (an
        // all-default MeetingConfig), so the wire params decode cleanly —
        // sending no params here yields jsonrpsee -32602 Invalid params.
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
        return try await sendRequestNoParams(
            method: "ipc.meeting.list"
        )
    }

    // MARK: - Private

    /// Core HTTP transport: sends encoded body, reads response, decodes result.
    private func sendRawRequest<R: Decodable>(
        body: Data
    ) async throws -> R {
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

    private func sendRequest<P: Encodable, R: Decodable>(
        method: String,
        params: P
    ) async throws -> R {
        let id = nextId
        nextId += 1
        let envelope = JsonRpcRequest(id: id, method: method, param: params)
        let body = try JSONEncoder().encode(envelope)
        return try await sendRawRequest(body: body)
    }

    /// Send request for methods that take no parameters.
    private func sendRequestNoParams<R: Decodable>(
        method: String
    ) async throws -> R {
        let id = nextId
        nextId += 1
        let envelope = JsonRpcRequestNoParams(id: id, method: method)
        let body = try JSONEncoder().encode(envelope)
        return try await sendRawRequest(body: body)
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
