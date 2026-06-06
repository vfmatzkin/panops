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
}

/// Tagged union over the event types we consume.
enum IpcEvent: Decodable {
    case jobDone(jobId: String, result: JobDoneResult)
    case jobError(jobId: String, error: JobErrorPayload)
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
        default:
            self = .unknown(type: type)
        }
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

struct JsonRpcResponse<R: Decodable>: Decodable {
    let jsonrpc: String
    let id: UInt64
    let result: R?
    let error: JsonRpcError?
}

struct JsonRpcError: Decodable {
    let code: Int
    let message: String
}

/// Empty params for RPCs that take no arguments.
struct EmptyParams: Encodable {
    init() {}
}

/// Outgoing params for `ipc.meeting.get`.
struct MeetingGetParams: Encodable {
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
struct MeetingSummary: Decodable {
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
