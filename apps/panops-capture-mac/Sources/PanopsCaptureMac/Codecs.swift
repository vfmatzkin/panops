import Foundation

/// Incoming params for `capture.start` / `capture.stop` (1-element positional
/// array, mirroring the ASR sidecar's `params: [..]` envelope). Every field is
/// optional because `capture.stop` carries only `meeting_id`.
struct CaptureParams: Decodable {
    let meetingId: String?
    let systemAudioPath: String?
    let micAudioPath: String?
    let screenshotsDir: String?
    let audioSources: String?            // "system_only" | "mic_only" | "system_and_mic"
    let screenshotIntervalMs: UInt64?
    let screenshotThreshold: Float?

    enum CodingKeys: String, CodingKey {
        case meetingId = "meeting_id"
        case systemAudioPath = "system_audio_path"
        case micAudioPath = "mic_audio_path"
        case screenshotsDir = "screenshots_dir"
        case audioSources = "audio_sources"
        case screenshotIntervalMs = "screenshot_interval_ms"
        case screenshotThreshold = "screenshot_threshold"
    }
}

/// JSON-RPC 2.0 request envelope. `id` is optional only to mirror the ASR
/// sidecar's parse-error handling; capture requests always carry one.
struct JsonRpcRequest: Decodable {
    let jsonrpc: String
    let id: UInt64?
    let method: String
    let params: [CaptureParams]
}

/// Result of `capture.start`.
struct StartedResult: Encodable {
    let startedAtMs: UInt64
    enum CodingKeys: String, CodingKey { case startedAtMs = "started_at_ms" }
}

/// Result of `capture.stop`. Each audio path is non-null exactly when its
/// source was requested via `audio_sources`.
struct StoppedResult: Encodable {
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

struct JsonRpcError: Encodable, Error {
    let code: Int
    let message: String
}

/// Empty placeholder result for error-only responses (lets the generic
/// `JsonRpcResponse<R>` carry an error without a real result type).
struct Empty: Encodable {}

/// JSON-RPC 2.0 response envelope. Custom `encode(to:)` writes an explicit
/// `null` id for parse-error responses (JSON-RPC 2.0 §4) instead of omitting
/// the key — matching the ASR sidecar's leak discipline.
struct JsonRpcResponse<R: Encodable>: Encodable {
    let id: UInt64?
    var result: R?
    var error: JsonRpcError?

    init(id: UInt64?, result: R? = nil, error: JsonRpcError? = nil) {
        self.id = id
        self.result = result
        self.error = error
    }

    enum CodingKeys: String, CodingKey {
        case jsonrpc, id, result, error
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode("2.0", forKey: .jsonrpc)
        if let id { try c.encode(id, forKey: .id) } else { try c.encodeNil(forKey: .id) }
        try c.encodeIfPresent(result, forKey: .result)
        try c.encodeIfPresent(error, forKey: .error)
    }
}
