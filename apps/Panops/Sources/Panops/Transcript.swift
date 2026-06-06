import Foundation

/// Transcript JSON shape matching the engine's output.
/// See spec appendix: handlers.rs writes transcript.json with this shape.
struct Transcript: Codable {
    let schemaVersion: String
    let model: String
    let audioPath: String?
    let audioDurationMs: UInt64?
    let diarized: Bool?
    let segments: [TranscriptSegment]

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case model
        case audioPath = "audio_path"
        case audioDurationMs = "audio_duration_ms"
        case diarized
        case segments
    }
}

struct TranscriptSegment: Codable, Hashable {
    let startMs: UInt64
    let endMs: UInt64
    let text: String
    let speaker: String?

    enum CodingKeys: String, CodingKey {
        case startMs = "start_ms"
        case endMs = "end_ms"
        case text
        case speaker
    }

    /// Format timestamp as [MM:SS–MM:SS].
    var timestampRange: String {
        let startMin = startMs / 60000
        let startSec = (startMs % 60000) / 1000
        let endMin = endMs / 60000
        let endSec = (endMs % 60000) / 1000
        return "[\(startMin):\(String(format: "%02d", startSec))–\(endMin):\(String(format: "%02d", endSec))]"
    }

    /// Format speaker label (e.g., "SPEAKER_01" → "Speaker 1").
    var speakerLabel: String {
        guard let speaker else { return "?" }
        // Strip "SPEAKER_" prefix and format
        if speaker.hasPrefix("SPEAKER_") {
            let num = speaker.dropFirst("SPEAKER_".count)
            return "Speaker \(num)"
        }
        return speaker
    }
}