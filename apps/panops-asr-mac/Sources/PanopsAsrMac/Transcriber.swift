import Foundation
import WhisperKit

/// Whisper special tokens that may appear in segment text. The probe
/// (slice 10 Task 3) confirmed segment.text includes tokens like
/// `<|startoftranscript|><|en|><|transcribe|><|0.00|>` mixed with the
/// transcribed words. Strip them at the adapter boundary so panops
/// downstream consumers see clean prose.
private let whisperSpecialTokenRegex = try! NSRegularExpression(
    pattern: #"<\|[^|>]*\|>"#,
    options: []
)

private func stripSpecialTokens(_ s: String) -> String {
    let range = NSRange(s.startIndex..<s.endIndex, in: s)
    let cleaned = whisperSpecialTokenRegex.stringByReplacingMatches(
        in: s, options: [], range: range, withTemplate: ""
    )
    return cleaned.trimmingCharacters(in: .whitespacesAndNewlines)
}

actor Transcriber {
    private let whisperKit: WhisperKit
    private let modelName: String

    private init(modelName: String, whisperKit: WhisperKit) {
        self.modelName = modelName
        self.whisperKit = whisperKit
    }

    /// Build a singleton Transcriber. Picks the smallest variant
    /// containing "tiny" from upstream HF; falls back to a hardcoded
    /// string if the list fetch fails (offline-ish path).
    static func makeShared() async throws -> Transcriber {
        let chosen: String
        do {
            let available = try await WhisperKit.fetchAvailableModels(
                from: "argmaxinc/whisperkit-coreml"
            )
            chosen = available.first(where: { $0.lowercased().contains("tiny") })
                ?? "openai_whisper-tiny"
        } catch {
            FileHandle.standardError.write(Data(
                "Transcriber.makeShared: fetchAvailableModels failed (\(error)); falling back to openai_whisper-tiny\n".utf8
            ))
            chosen = "openai_whisper-tiny"
        }
        FileHandle.standardError.write(Data(
            "Transcriber.makeShared: loading variant \(chosen)\n".utf8
        ))
        let kit = try await WhisperKit(WhisperKitConfig(model: chosen))
        return Transcriber(modelName: chosen, whisperKit: kit)
    }

    func transcribe(audioPath: String, languageHint: String?) async throws -> WireTranscript {
        let opts = DecodingOptions(language: languageHint)
        let results = try await whisperKit.transcribe(
            audioPath: audioPath, decodeOptions: opts
        )
        var segments: [WireSegment] = []
        var lastEndMs: UInt64 = 0
        for r in results {
            // `language` is on the result, not per-segment — fan out
            // the top-level language across this result's segments.
            let resultLanguage = r.language
            for s in r.segments {
                let startMs = UInt64(max(0, s.start * 1000.0))
                let endMs = UInt64(max(0, s.end * 1000.0))
                lastEndMs = max(lastEndMs, endMs)
                segments.append(WireSegment(
                    startMs: startMs,
                    endMs: endMs,
                    text: stripSpecialTokens(s.text),
                    languageDetected: resultLanguage,
                    confidence: 1.0,
                    isPartial: false,
                    speakerId: nil
                ))
            }
        }
        return WireTranscript(
            model: "whisperkit-\(modelName)",
            audioPath: audioPath,
            audioDurationMs: lastEndMs,
            segments: segments
        )
    }
}
