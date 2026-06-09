import Foundation

/// Transcript JSON shape matching the engine's `panops_core::segment::Transcript`.
/// Keep field names and types in sync with crates/panops-core/src/segment.rs.
struct Transcript: Codable {
    let schemaVersion: UInt32
    let model: String
    let audioPath: String
    let audioDurationMs: UInt64
    let diarized: Bool
    let segments: [TranscriptSegment]

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case model
        case audioPath = "audio_path"
        case audioDurationMs = "audio_duration_ms"
        case diarized
        case segments
    }

    init(
        schemaVersion: UInt32,
        model: String,
        audioPath: String,
        audioDurationMs: UInt64,
        diarized: Bool,
        segments: [TranscriptSegment]
    ) {
        self.schemaVersion = schemaVersion
        self.model = model
        self.audioPath = audioPath
        self.audioDurationMs = audioDurationMs
        self.diarized = diarized
        self.segments = segments
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        schemaVersion = try container.decode(UInt32.self, forKey: .schemaVersion)
        model = try container.decode(String.self, forKey: .model)
        audioPath = try container.decode(String.self, forKey: .audioPath)
        audioDurationMs = try container.decode(UInt64.self, forKey: .audioDurationMs)
        diarized = try container.decodeIfPresent(Bool.self, forKey: .diarized) ?? false
        segments = try container.decode([TranscriptSegment].self, forKey: .segments)
    }
}

struct TranscriptSegment: Codable, Hashable {
    let startMs: UInt64
    let endMs: UInt64
    let text: String
    let languageDetected: String?
    let confidence: Float
    let speakerId: UInt32?

    enum CodingKeys: String, CodingKey {
        case startMs = "start_ms"
        case endMs = "end_ms"
        case text
        case languageDetected = "language_detected"
        case confidence
        case speakerId = "speaker_id"
    }

    init(
        startMs: UInt64,
        endMs: UInt64,
        text: String,
        languageDetected: String?,
        confidence: Float,
        speakerId: UInt32?
    ) {
        self.startMs = startMs
        self.endMs = endMs
        self.text = text
        self.languageDetected = languageDetected
        self.confidence = confidence
        self.speakerId = speakerId
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        startMs = try container.decode(UInt64.self, forKey: .startMs)
        endMs = try container.decode(UInt64.self, forKey: .endMs)
        text = try container.decode(String.self, forKey: .text)
        languageDetected = try container.decodeIfPresent(String.self, forKey: .languageDetected)
        confidence = try container.decode(Float.self, forKey: .confidence)
        speakerId = try container.decodeIfPresent(UInt32.self, forKey: .speakerId)
    }

    /// Format timestamp as [MM:SS–MM:SS].
    var timestampRange: String {
        let startMin = startMs / 60000
        let startSec = (startMs % 60000) / 1000
        let endMin = endMs / 60000
        let endSec = (endMs % 60000) / 1000
        return "[\(startMin):\(String(format: "%02d", startSec))–\(endMin):\(String(format: "%02d", endSec))]"
    }

    /// Format engine speaker ids like the markdown exporter: 0 → "Speaker 1".
    var speakerLabel: String? {
        guard let speakerId else { return nil }
        return "Speaker \(UInt64(speakerId) + 1)"
    }
}
