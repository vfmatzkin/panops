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
    // `nonisolated(unsafe)`: WhisperKit is not Sendable, so calling its
    // nonisolated `transcribe` from this actor would "send" actor-isolated
    // state across isolation (a hard error under newer Swift toolchains).
    // Safety does NOT come from actor isolation — an actor can re-enter
    // `transcribe` at its `await` suspension, so isolation alone would not
    // prevent concurrent use of the handle. It comes from the caller: the
    // sidecar's stdio loop (main.swift) reads and fully awaits one request
    // before reading the next, so `transcribe` is single-flight in practice.
    // Revisit (e.g. a serial executor) if that loop ever becomes concurrent.
    private nonisolated(unsafe) let whisperKit: WhisperKit
    private let modelName: String

    private init(modelName: String, whisperKit: WhisperKit) {
        self.modelName = modelName
        self.whisperKit = whisperKit
    }

    /// Build a singleton Transcriber. Picks `openai_whisper-base`
    /// (multilingual, ~150 MB) by default — tiny's Spanish recall is
    /// too low to pass the AsrProvider conformance suite on `es_30s.wav`.
    /// Override via env var `PANOPS_WHISPERKIT_MODEL`. Falls back to a
    /// hardcoded string if `fetchAvailableModels` fails (offline path).
    static func makeShared() async throws -> Transcriber {
        let preferred = ProcessInfo.processInfo.environment["PANOPS_WHISPERKIT_MODEL"]
            ?? "openai_whisper-base"
        let chosen: String
        do {
            let available = try await WhisperKit.fetchAvailableModels(
                from: "argmaxinc/whisperkit-coreml"
            )
            chosen = available.first(where: { $0 == preferred })
                ?? available.first(where: { $0.contains(preferred) })
                ?? preferred
        } catch {
            FileHandle.standardError.write(Data(
                "Transcriber.makeShared: fetchAvailableModels failed (\(error)); using \(preferred)\n".utf8
            ))
            chosen = preferred
        }
        FileHandle.standardError.write(Data(
            "Transcriber.makeShared: loading variant \(chosen)\n".utf8
        ))
        let kit = try await WhisperKit(WhisperKitConfig(model: chosen))
        return Transcriber(modelName: chosen, whisperKit: kit)
    }

    func transcribe(audioPath: String, languageHint: String?) async throws -> WireTranscript {
        // Force language when explicitly provided (skip auto-detect).
        // When no hint, explicitly enable language detection (fixes
        // tiny/base misdetecting Spanish as English per issue #125).
        let opts = DecodingOptions(
            language: languageHint,
            detectLanguage: languageHint == nil ? true : false
        )
        let results = try await whisperKit.transcribe(
            audioPath: audioPath, decodeOptions: opts
        )
        var segments: [WireSegment] = []
        var lastEndMs: UInt64 = 0
        var prevEnd: UInt64 = 0
        for r in results {
            // `language` is on the result, not per-segment — fan out
            // the top-level language across this result's segments.
            let resultLanguage = r.language
            for s in r.segments {
                let rawStartMs = UInt64(max(0, s.start * 1000.0))
                let rawEndMs = UInt64(max(0, s.end * 1000.0))
                // WhisperKit's segment timestamps can have 1-ms rounding
                // overlaps with the previous segment. The panops conformance
                // suite (and downstream stitching in slice 07/08) requires
                // strictly non-overlapping segments. Bump `start_ms` up to
                // `prev_end_ms` to enforce monotonic timestamps without
                // dropping any audio coverage.
                let startMs = max(rawStartMs, prevEnd)
                let endMs = max(startMs, rawEndMs)
                prevEnd = endMs
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
