import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
import Testing
@testable import PanopsCaptureMac

/// Covers the pure recording.mov path-derivation plus the AVAssetWriter setup
/// and finalize, both of which run without screen/mic/TCC. The live SCStream →
/// mux path is exercised by the manual Mac smoke.
struct VideoWriterTests {
    // MARK: - Output URL derivation

    @Test func outputURLPrefersAudioParent() {
        let url = VideoWriter.outputURL(
            systemAudioPath: "/tmp/meeting/system.wav",
            micAudioPath: "/tmp/meeting/mic.wav",
            screenshotsDir: "/tmp/meeting/screenshots"
        )
        #expect(url?.path == "/tmp/meeting/recording.mov")
    }

    @Test func outputURLFallsBackToMicThenScreenshots() {
        #expect(
            VideoWriter.outputURL(systemAudioPath: nil, micAudioPath: "/m/mic.wav", screenshotsDir: nil)?
                .path == "/m/recording.mov")
        // No audio path: derive the meeting dir from the screenshots subdir.
        #expect(
            VideoWriter.outputURL(systemAudioPath: nil, micAudioPath: nil, screenshotsDir: "/m/screenshots")?
                .path == "/m/recording.mov")
    }

    @Test func outputURLNilWhenNothingIdentifiesMeetingDir() {
        #expect(VideoWriter.outputURL(systemAudioPath: nil, micAudioPath: nil, screenshotsDir: nil) == nil)
    }

    // MARK: - Writer setup + finalize

    @Test func initSucceedsAndFinishWithoutFramesIsIdempotent() async throws {
        let url = Self.tempMovURL()
        defer { try? FileManager.default.removeItem(at: url) }
        let writer = try VideoWriter(url: url, width: 320, height: 240)
        // No frames → session never starts → finish is a no-op. Calling it
        // twice must not crash (the finishing/session guards cover re-entry).
        await writer.finish()
        await writer.finish()
        #expect(writer.url == url)
    }

    @Test func appendingFramesProducesPlayableVideoTrack() async throws {
        let url = Self.tempMovURL()
        defer { try? FileManager.default.removeItem(at: url) }
        let writer = try VideoWriter(url: url, width: 64, height: 48)
        for i in 0..<10 {
            if let frame = Self.makeFrame(width: 64, height: 48, pts: CMTime(value: Int64(i), timescale: 30)) {
                writer.appendVideo(frame)
            }
        }
        await writer.finish()

        #expect(FileManager.default.fileExists(atPath: url.path))
        let tracks = try await AVURLAsset(url: url).loadTracks(withMediaType: .video)
        #expect(!tracks.isEmpty)
    }

    // MARK: - Fixtures

    private static func tempMovURL() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("panops-vid-\(UUID().uuidString).mov")
    }

    /// Build a synthetic IOSurface-backed BGRA video sample buffer — no TCC,
    /// display, or SCStream needed.
    static func makeFrame(width: Int, height: Int, pts: CMTime) -> CMSampleBuffer? {
        var pixelBuffer: CVPixelBuffer?
        let attrs: [String: Any] = [kCVPixelBufferIOSurfacePropertiesKey as String: [:]]
        guard
            CVPixelBufferCreate(
                kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA,
                attrs as CFDictionary, &pixelBuffer) == kCVReturnSuccess,
            let pb = pixelBuffer
        else { return nil }

        var formatDesc: CMVideoFormatDescription?
        guard
            CMVideoFormatDescriptionCreateForImageBuffer(
                allocator: kCFAllocatorDefault, imageBuffer: pb, formatDescriptionOut: &formatDesc) == noErr,
            let fmt = formatDesc
        else { return nil }

        var timing = CMSampleTimingInfo(
            duration: CMTime(value: 1, timescale: 30), presentationTimeStamp: pts, decodeTimeStamp: .invalid)
        var sampleBuffer: CMSampleBuffer?
        guard
            CMSampleBufferCreateReadyWithImageBuffer(
                allocator: kCFAllocatorDefault, imageBuffer: pb, formatDescription: fmt,
                sampleTiming: &timing, sampleBufferOut: &sampleBuffer) == noErr
        else { return nil }
        return sampleBuffer
    }
}
