import Foundation

/// Response from `ipc.server.info`.
struct ServerInfo: Decodable, Equatable {
    let llm: LlmInfo
}

/// Active LLM provider advertised by the engine.
struct LlmInfo: Decodable, Equatable {
    let provider: String
    let model: String
    let local: Bool
}

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

    /// Single source of truth for how an audio choice reads to the user. Used by
    /// the New Recording picker and the recording-screen capture chip so the two
    /// can't drift apart.
    var displayLabel: String {
        switch self {
        case .systemAndMic: return "System + Mic"
        case .micOnly: return "Mic only"
        case .systemOnly: return "System only"
        }
    }

    /// SF Symbol paired with `displayLabel` in the capture chip.
    var icon: String {
        switch self {
        case .systemAndMic: return "waveform"
        case .micOnly: return "mic"
        case .systemOnly: return "speaker.wave.2"
        }
    }
}

/// Outgoing params for `ipc.meeting.start`. Mirrors `panops-protocol::MeetingConfig`.
/// Both fields optional; the engine applies defaults (title="", language="auto").
/// `nil` fields are omitted by `JSONEncoder`, so an all-`nil` config encodes to
/// `{}` — identical to the previous `EmptyParams()` path.
struct MeetingConfig: Encodable {
    let title: String?
    let language: String?

    // Explicit snake_case keys per the wire convention (see MeetingSummary).
    // Both fields are single-word today, so this matches the current wire, but
    // it pins the mapping so a future multi-word field can't silently drift from
    // the engine's snake_case contract. Optionals still encode via
    // `encodeIfPresent`, so an all-nil config stays `{}`.
    enum CodingKeys: String, CodingKey {
        case title
        case language
    }
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
///
/// Base wire contract: id, title, started_at, duration_ms (all required).
/// `language`, `ended_at`, and `has_notes` are decode-if-present: the engine
/// change that adds them lands separately, so older payloads (and the existing
/// codec test) still decode — `language` defaults to "", `endedAt` to nil,
/// `hasNotes` to false.
struct MeetingSummary: Decodable {
    let id: String
    let title: String
    let startedAt: String
    let durationMs: UInt64
    let language: String
    let endedAt: String?
    let hasNotes: Bool

    enum CodingKeys: String, CodingKey {
        case id
        case title
        case startedAt = "started_at"
        case durationMs = "duration_ms"
        case language
        case endedAt = "ended_at"
        case hasNotes = "has_notes"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        title = try c.decode(String.self, forKey: .title)
        startedAt = try c.decode(String.self, forKey: .startedAt)
        durationMs = try c.decode(UInt64.self, forKey: .durationMs)
        language = try c.decodeIfPresent(String.self, forKey: .language) ?? ""
        endedAt = try c.decodeIfPresent(String.self, forKey: .endedAt)
        hasNotes = try c.decodeIfPresent(Bool.self, forKey: .hasNotes) ?? false
    }

    // Memberwise init retained for tests/previews constructing summaries directly.
    init(
        id: String,
        title: String,
        startedAt: String,
        durationMs: UInt64,
        language: String = "",
        endedAt: String? = nil,
        hasNotes: Bool = false
    ) {
        self.id = id
        self.title = title
        self.startedAt = startedAt
        self.durationMs = durationMs
        self.language = language
        self.endedAt = endedAt
        self.hasNotes = hasNotes
    }
}
