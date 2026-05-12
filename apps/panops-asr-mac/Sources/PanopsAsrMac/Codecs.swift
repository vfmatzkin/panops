import Foundation

/// Mirrors `panops_core::Segment` on the wire.
struct WireSegment: Encodable {
    let startMs: UInt64
    let endMs: UInt64
    let text: String
    let languageDetected: String?
    let confidence: Float
    let isPartial: Bool
    let speakerId: UInt32?

    enum CodingKeys: String, CodingKey {
        case startMs = "start_ms"
        case endMs = "end_ms"
        case text
        case languageDetected = "language_detected"
        case confidence
        case isPartial = "is_partial"
        case speakerId = "speaker_id"
    }
}

/// Mirrors `panops_core::Transcript` on the wire.
struct WireTranscript: Encodable {
    let schemaVersion: UInt32 = 2
    let model: String
    let audioPath: String
    let audioDurationMs: UInt64
    let diarized: Bool = false
    let segments: [WireSegment]

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case model
        case audioPath = "audio_path"
        case audioDurationMs = "audio_duration_ms"
        case diarized
        case segments
    }
}

/// Incoming params for `asr.transcribe` (1-element positional array).
struct TranscribeParams: Decodable {
    let audio: String
    let sampleRate: UInt32
    let languageHint: String?

    enum CodingKeys: String, CodingKey {
        case audio
        case sampleRate = "sample_rate"
        case languageHint = "language_hint"
    }
}

/// JSON-RPC 2.0 envelopes.
struct JsonRpcRequest: Decodable {
    let jsonrpc: String
    let id: UInt64
    let method: String
    let params: [TranscribeParams]
}

struct JsonRpcResponse: Encodable {
    let jsonrpc: String
    let id: UInt64
    let result: WireTranscript?
    let error: JsonRpcError?

    init(id: UInt64, result: WireTranscript? = nil, error: JsonRpcError? = nil) {
        self.jsonrpc = "2.0"
        self.id = id
        self.result = result
        self.error = error
    }
}

struct JsonRpcError: Encodable {
    let code: Int
    let message: String
}
