import CoreImage
import CoreMedia
import Foundation
@preconcurrency import AVFoundation

/// One extracted screenshot's metadata, emitted as JSON by
/// `--extract-screenshots`. `timestamp_ms` is the frame's position in the
/// recording (0 = start) — the same anchor the live `Screenshotter` uses for
/// its filenames.
struct ExtractedScreenshot: Encodable {
    let timestampMs: UInt64
    let path: String

    enum CodingKeys: String, CodingKey {
        case timestampMs = "timestamp_ms"
        case path
    }
}

enum ScreenshotExtractionError: Error, CustomStringConvertible {
    case noVideoTrack(String)

    var description: String {
        switch self {
        case .noVideoTrack(let path): return "no video track in \(path)"
        }
    }
}

/// Decodes frames from a recorded `.mov`/`.mp4` at `intervalMs` cadence, runs
/// the shared `ChangeDetector`, and writes the kept frames as time-anchored
/// JPEGs into `outDir` — the SAME naming + JPEG format the live `Screenshotter`
/// produces. The one-shot counterpart to live screenshotting: identical
/// detector + encoding, a decoded-video frame source instead of live SCStream
/// frames. Stage A is purely additive — nothing calls this from the pipeline
/// yet (that is Stage B's engine rewire).
enum ScreenshotExtractor {
    static func extract(
        movPath: String,
        outDir: String,
        intervalMs: UInt64,
        threshold: Float
    ) async throws -> [ExtractedScreenshot] {
        let movURL = URL(fileURLWithPath: movPath)
        let asset = AVURLAsset(url: movURL)

        // A video track must exist; otherwise there is nothing to extract.
        guard try await !asset.loadTracks(withMediaType: .video).isEmpty else {
            throw ScreenshotExtractionError.noVideoTrack(movPath)
        }

        let outDirURL = URL(fileURLWithPath: outDir)
        try FileManager.default.createDirectory(at: outDirURL, withIntermediateDirectories: true)

        let times = sampleTimes(durationMs: await durationMs(of: asset), intervalMs: intervalMs)

        let generator = AVAssetImageGenerator(asset: asset)
        generator.appliesPreferredTrackTransform = true
        // Zero tolerance ⇒ the exact frame on screen at each requested instant,
        // so the JPEG filename's timestamp matches the recording. In-range marks
        // always resolve to a frame; only marks past the end fail (skipped).
        generator.requestedTimeToleranceBefore = .zero
        generator.requestedTimeToleranceAfter = .zero

        let detector = ChangeDetector(intervalMs: intervalMs, threshold: threshold)
        let ciContext = CIContext()
        var kept: [ExtractedScreenshot] = []

        for await result in generator.images(for: times) {
            let cg: CGImage
            let actualMs: UInt64
            do {
                cg = try result.image
                actualMs = UInt64(max(0, try result.actualTime.seconds * 1000))
            } catch {
                continue   // no decodable frame at this mark (e.g. past the end)
            }
            let keep = detector.shouldKeep(atSampleMs: actualMs) {
                featurePrint(cgImage: cg)
            }
            guard keep else { continue }

            let url = outDirURL.appendingPathComponent(String(format: "%09d.jpg", actualMs))
            guard let data = encodeScreenshotJPEG(CIImage(cgImage: cg), using: ciContext) else { continue }
            do {
                try data.write(to: url)
                kept.append(ExtractedScreenshot(timestampMs: actualMs, path: url.path))
            } catch {
                FileHandle.standardError.write(Data("extract jpeg write failed: \(error)\n".utf8))
            }
        }
        return kept
    }

    /// Asset duration in ms, or 0 when indefinite/unavailable.
    private static func durationMs(of asset: AVURLAsset) async -> Int64 {
        guard let duration = try? await asset.load(.duration), duration.isNumeric else { return 0 }
        return Int64(max(0, duration.seconds * 1000))
    }

    /// Sample marks at 0, interval, 2·interval, … up to (and including) the
    /// asset duration. Always includes t=0, so even a zero/unknown-duration
    /// asset yields at least the first frame.
    private static func sampleTimes(durationMs: Int64, intervalMs: UInt64) -> [CMTime] {
        let step = Int64(max(intervalMs, 1))
        var times: [CMTime] = []
        var t: Int64 = 0
        repeat {
            times.append(CMTime(value: t, timescale: 1000))
            t += step
        } while t <= durationMs
        return times
    }
}
