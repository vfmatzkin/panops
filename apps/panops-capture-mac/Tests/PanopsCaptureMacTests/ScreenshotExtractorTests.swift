import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
import Testing
@testable import PanopsCaptureMac

/// Exercises the `--extract-screenshots` decode → detect → write → JSON path
/// end to end: a synthetic `.mov` built in-test (deterministic, no fixture
/// dependency) and the git-tracked `tests/fixtures/video/screen_60s.mp4`. Both
/// assert ≥1 JPEG on disk plus valid, self-consistent JSON metadata. The
/// detector's keep/skip logic itself is covered in `ChangeDetectorTests`.
struct ScreenshotExtractorTests {
    @Test func extractsFromSyntheticMovWritesJpegsAndJSON() async throws {
        let movURL = Self.tempURL("mov")
        let outDir = Self.tempURL("dir")
        defer {
            try? FileManager.default.removeItem(at: movURL)
            try? FileManager.default.removeItem(at: outDir)
        }
        // Distinctly-colored solid frames 250ms apart (4 fps, ~1s of video).
        try await Self.writeSyntheticMov(
            to: movURL, width: 64, height: 48,
            colors: [.red, .green, .blue, .yellow], fps: 4)

        let shots = try await ScreenshotExtractor.extract(
            movPath: movURL.path, outDir: outDir.path, intervalMs: 250, threshold: 0.15)

        // The change detector always keeps the first frame.
        #expect(shots.count >= 1)

        // Every metadata entry points at a real `.jpg`, named by its timestamp.
        for shot in shots {
            #expect(shot.path.hasSuffix(".jpg"))
            #expect(FileManager.default.fileExists(atPath: shot.path))
            #expect(shot.path.hasSuffix(String(format: "%09d.jpg", shot.timestampMs)))
        }
        // Metadata count matches the JPEGs actually written to out_dir.
        let jpegs = try FileManager.default.contentsOfDirectory(atPath: outDir.path)
            .filter { $0.hasSuffix(".jpg") }
        #expect(jpegs.count == shots.count)

        // The emitted JSON is the wire shape the engine will parse.
        let data = try JSONEncoder().encode(shots)
        let decoded = try JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        #expect(decoded?.count == shots.count)
        #expect(decoded?.allSatisfy { $0["timestamp_ms"] != nil && $0["path"] != nil } == true)
    }

    @Test func extractsFromFixtureMov() async throws {
        // The fixture is committed to the repo, so it must be present.
        let fixture = try #require(Self.fixtureVideoURL(), "missing tests/fixtures/video/screen_60s.mp4")
        let outDir = Self.tempURL("dir")
        defer { try? FileManager.default.removeItem(at: outDir) }

        let shots = try await ScreenshotExtractor.extract(
            movPath: fixture.path, outDir: outDir.path, intervalMs: 1000, threshold: 0.15)

        #expect(shots.count >= 1)
        for shot in shots {
            #expect(FileManager.default.fileExists(atPath: shot.path))
        }
        // Metadata round-trips to JSON.
        let data = try JSONEncoder().encode(shots)
        let decoded = try JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        #expect(decoded?.count == shots.count)
    }

    // MARK: - Fixtures

    private static func tempURL(_ ext: String) -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("panops-extract-\(UUID().uuidString).\(ext)")
    }

    /// Repo `tests/fixtures` dir, located from this source file's path so it
    /// resolves regardless of the test's working directory. `PANOPS_FIXTURES_DIR`
    /// overrides it (matching the Mac-shell smoke test).
    private static func fixturesDir() -> URL {
        if let override = ProcessInfo.processInfo.environment["PANOPS_FIXTURES_DIR"] {
            return URL(fileURLWithPath: override)
        }
        // <repo>/apps/panops-capture-mac/Tests/PanopsCaptureMacTests/<thisfile>
        return URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // PanopsCaptureMacTests
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // panops-capture-mac
            .deletingLastPathComponent()   // apps
            .deletingLastPathComponent()   // repo root
            .appendingPathComponent("tests/fixtures")
    }

    private static func fixtureVideoURL() -> URL? {
        let url = fixturesDir().appendingPathComponent("video/screen_60s.mp4")
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    /// A solid BGRA fill color for synthetic frames.
    struct SolidColor {
        let b, g, r, a: UInt8
        static let red = SolidColor(b: 0, g: 0, r: 255, a: 255)
        static let green = SolidColor(b: 0, g: 255, r: 0, a: 255)
        static let blue = SolidColor(b: 255, g: 0, r: 0, a: 255)
        static let yellow = SolidColor(b: 0, g: 255, r: 255, a: 255)
    }

    /// Write `colors.count` solid-color frames into a playable `.mov` via the
    /// project's `VideoWriter` — no TCC, display, or SCStream needed.
    private static func writeSyntheticMov(
        to url: URL, width: Int, height: Int, colors: [SolidColor], fps: Int32
    ) async throws {
        let writer = try VideoWriter(url: url, width: width, height: height)
        for (i, color) in colors.enumerated() {
            let pts = CMTime(value: Int64(i), timescale: fps)
            if let frame = makeSolidFrame(
                width: width, height: height, color: color, pts: pts,
                duration: CMTime(value: 1, timescale: fps)) {
                writer.appendVideo(frame)
            }
        }
        await writer.finish()
    }

    /// A solid-color BGRA IOSurface-backed sample buffer (same construction as
    /// `VideoWriterTests.makeFrame`, filled with one color).
    static func makeSolidFrame(
        width: Int, height: Int, color: SolidColor, pts: CMTime, duration: CMTime
    ) -> CMSampleBuffer? {
        var pixelBuffer: CVPixelBuffer?
        let attrs: [String: Any] = [kCVPixelBufferIOSurfacePropertiesKey as String: [:]]
        guard
            CVPixelBufferCreate(
                kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA,
                attrs as CFDictionary, &pixelBuffer) == kCVReturnSuccess,
            let pb = pixelBuffer
        else { return nil }

        CVPixelBufferLockBaseAddress(pb, [])
        if let base = CVPixelBufferGetBaseAddress(pb) {
            let bytesPerRow = CVPixelBufferGetBytesPerRow(pb)
            let ptr = base.assumingMemoryBound(to: UInt8.self)
            for y in 0..<height {
                for x in 0..<width {
                    let o = y * bytesPerRow + x * 4
                    ptr[o + 0] = color.b
                    ptr[o + 1] = color.g
                    ptr[o + 2] = color.r
                    ptr[o + 3] = color.a
                }
            }
        }
        CVPixelBufferUnlockBaseAddress(pb, [])

        var formatDesc: CMVideoFormatDescription?
        guard
            CMVideoFormatDescriptionCreateForImageBuffer(
                allocator: kCFAllocatorDefault, imageBuffer: pb, formatDescriptionOut: &formatDesc) == noErr,
            let fmt = formatDesc
        else { return nil }

        var timing = CMSampleTimingInfo(
            duration: duration, presentationTimeStamp: pts, decodeTimeStamp: .invalid)
        var sampleBuffer: CMSampleBuffer?
        guard
            CMSampleBufferCreateReadyWithImageBuffer(
                allocator: kCFAllocatorDefault, imageBuffer: pb, formatDescription: fmt,
                sampleTiming: &timing, sampleBufferOut: &sampleBuffer) == noErr
        else { return nil }
        return sampleBuffer
    }
}
