import Foundation

/// Outgoing params for `ipc.notes.generate`.
struct NotesGenerateParams: Encodable {
    let audio: String
    let dialect: String?
    let language: String?
    let llmProvider: String?
    let llmModel: String?
    let noDiarize: Bool?
    let meetingId: String?

    enum CodingKeys: String, CodingKey {
        case audio
        case dialect
        case language
        case llmProvider = "llm_provider"
        case llmModel = "llm_model"
        case noDiarize = "no_diarize"
        case meetingId = "meeting_id"
    }
}

/// Synchronous response from `ipc.notes.generate`.
struct NotesGenerateResult: Decodable {
    let jobId: String

    enum CodingKeys: String, CodingKey {
        case jobId = "job_id"
    }
}

/// `job.done` event payload.
struct JobDoneResult: Decodable {
    let primaryFile: String
    let assets: [String]
    let meetingId: String

    enum CodingKeys: String, CodingKey {
        case primaryFile = "primary_file"
        case assets
        case meetingId = "meeting_id"
    }
}

/// `job.error` event payload.
struct JobErrorPayload: Decodable {
    let kind: String
    let message: String

    private enum CodingKeys: String, CodingKey {
        case kind
        case message
    }

    init(kind: String, message: String) {
        self.kind = kind
        self.message = message
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(String.self, forKey: .kind)
        message = try c.decodeIfPresent(String.self, forKey: .message) ?? ""
    }
}

/// `job.progress` event payload.
struct JobProgressEvent: Decodable, Equatable {
    let jobId: String
    let stage: String
    let current: Int?
    let total: Int?
    let message: String?

    enum CodingKeys: String, CodingKey {
        case jobId = "job_id"
        case stage
        case current
        case total
        case message
    }
}

/// Tagged union over the event types we consume.
enum IpcEvent: Decodable {
    case jobDone(jobId: String, result: JobDoneResult)
    case jobError(jobId: String, error: JobErrorPayload)
    case jobProgress(JobProgressEvent)
    case unknown(type: String)

    private enum CodingKeys: String, CodingKey {
        case type
        case jobId = "job_id"
        case result
        case error
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = try c.decode(String.self, forKey: .type)
        switch type {
        case "job.done":
            let jobId = try c.decode(String.self, forKey: .jobId)
            let result = try c.decode(JobDoneResult.self, forKey: .result)
            self = .jobDone(jobId: jobId, result: result)
        case "job.error":
            let jobId = try c.decode(String.self, forKey: .jobId)
            let error = try c.decode(JobErrorPayload.self, forKey: .error)
            self = .jobError(jobId: jobId, error: error)
        case "job.progress":
            self = .jobProgress(try JobProgressEvent(from: decoder))
        default:
            self = .unknown(type: type)
        }
    }
}

/// Audio source selection for `ipc.recording.start`.
enum AudioSourcesWire: String, Codable {
    case systemOnly = "system_only"
    case micOnly = "mic_only"
    case systemAndMic = "system_and_mic"
}

/// Outgoing params for `ipc.recording.start`.
struct RecordingStartParams: Encodable {
    let meetingId: String
    let audioSources: AudioSourcesWire
    let screenshotIntervalMs: UInt64
    let screenshotThreshold: Float

    enum CodingKeys: String, CodingKey {
        case meetingId = "meeting_id"
        case audioSources = "audio_sources"
        case screenshotIntervalMs = "screenshot_interval_ms"
        case screenshotThreshold = "screenshot_threshold"
    }
}

/// Synchronous response from `ipc.recording.start`.
struct RecordingAccepted: Decodable {
    let recordingId: String

    enum CodingKeys: String, CodingKey {
        case recordingId = "recording_id"
    }
}

/// Outgoing params for `ipc.recording.stop`.
struct RecordingStopParams: Encodable {
    let recordingId: String

    enum CodingKeys: String, CodingKey {
        case recordingId = "recording_id"
    }
}

/// Synchronous response from `ipc.recording.stop`.
struct RecordingStopped: Decodable {
    let systemAudioPath: String?
    let micAudioPath: String?
    let screenshotPaths: [String]
    let durationMs: UInt64

    enum CodingKeys: String, CodingKey {
        case systemAudioPath = "system_audio_path"
        case micAudioPath = "mic_audio_path"
        case screenshotPaths = "screenshot_paths"
        case durationMs = "duration_ms"
    }
}

/// Minimal JSON-RPC 2.0 envelopes (request and response).
/// jsonrpsee uses positional params: each method takes exactly one
/// argument, wrapped in a 1-element array on the wire.
struct JsonRpcRequest<P: Encodable>: Encodable {
    let jsonrpc: String
    let id: UInt64
    let method: String
    let params: [P]

    init(id: UInt64, method: String, param: P) {
        self.jsonrpc = "2.0"
        self.id = id
        self.method = method
        self.params = [param]
    }
}

/// JSON-RPC request for methods that take no parameters.
/// Per JSON-RPC 2.0 spec, params field is omitted entirely.
struct JsonRpcRequestNoParams: Encodable {
    let jsonrpc: String
    let id: UInt64
    let method: String

    init(id: UInt64, method: String) {
        self.jsonrpc = "2.0"
        self.id = id
        self.method = method
    }
}

struct JsonRpcResponse<R: Decodable>: Decodable {
    let jsonrpc: String
    let id: UInt64
    let result: R?
    let error: JsonRpcError?
}

struct JsonRpcVoidResponse: Decodable {
    let jsonrpc: String
    let id: UInt64
    let hasResult: Bool
    let error: JsonRpcError?

    enum CodingKeys: String, CodingKey {
        case jsonrpc
        case id
        case result
        case error
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        jsonrpc = try container.decode(String.self, forKey: .jsonrpc)
        id = try container.decode(UInt64.self, forKey: .id)
        hasResult = container.contains(.result)
        error = try container.decodeIfPresent(JsonRpcError.self, forKey: .error)
    }
}

struct JsonRpcError: Decodable {
    let code: Int
    let message: String
}

/// jsonrpsee subscription id — numeric by default; accept a string too.
/// Only its presence matters here (single subscription), so the value
/// is decoded flexibly and not otherwise used.
enum SubscriptionId: Decodable {
    case number(Int)
    case string(String)
    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let n = try? c.decode(Int.self) { self = .number(n); return }
        if let s = try? c.decode(String.self) { self = .string(s); return }
        throw DecodingError.typeMismatch(
            SubscriptionId.self,
            .init(codingPath: decoder.codingPath,
                  debugDescription: "subscription id is neither number nor string")
        )
    }
}

/// JSON-RPC 2.0 notification envelope for WebSocket events.
/// Wire format: {"jsonrpc":"2.0","method":"events","params":{"subscription":<id>,"result":<Event>}}
/// The Event payload is in params.result.
struct JsonRpcNotification: Decodable {
    let jsonrpc: String
    let method: String
    let params: NotificationParams

    struct NotificationParams: Decodable {
        // `subscription` (jsonrpsee sub id) is present on the wire but unused
        // here; omit it so its numeric type can't break event decoding.
        let result: IpcEvent
    }
}

/// Empty params for RPCs that take no arguments.
struct EmptyParams: Encodable {
    init() {}
}

/// Outgoing params for `ipc.meeting.get`.
struct MeetingGetParams: Encodable {
    let id: String
}

/// Outgoing params for `ipc.meeting.stop`.
struct MeetingStopParams: Encodable {
    let id: String
}

/// Outgoing params for `ipc.meeting.delete`.
struct MeetingDeleteParams: Encodable {
    let id: String
}

/// Response from `ipc.meeting.get`. Mirrors `panops-protocol::Meeting`.
/// `dir_path` is where notes.md eventually lands; slice 09 polls
/// `<dirPath>/notes.md` on disk to detect completion (the IPC's
/// Meeting type doesn't include note-file metadata).
struct Meeting: Decodable {
    let id: String
    let title: String
    let startedAt: String
    let endedAt: String?
    let durationMs: UInt64?
    let language: String
    let dirPath: String

    enum CodingKeys: String, CodingKey {
        case id
        case title
        case startedAt = "started_at"
        case endedAt = "ended_at"
        case durationMs = "duration_ms"
        case language
        case dirPath = "dir_path"
    }
}

/// Summary item from `ipc.meeting.list`. Mirrors `panops-protocol::MeetingSummary`.
/// Wire contract: ONLY 4 fields (id, title, started_at, duration_ms) — all required.
struct MeetingSummary: Decodable {
    let id: String
    let title: String
    let startedAt: String
    let durationMs: UInt64

    enum CodingKeys: String, CodingKey {
        case id
        case title
        case startedAt = "started_at"
        case durationMs = "duration_ms"
    }
}
