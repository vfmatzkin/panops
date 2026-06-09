import CoreImage
import CoreMedia
import Foundation
import ScreenCaptureKit

/// Samples `SCStream` `.screen` frames, dedups near-duplicates via the shared
/// `ChangeDetector` (Vision feature print + cosine distance vs the last *kept*
/// frame, below `threshold` ⇒ drop), and writes the kept ones as time-anchored
/// JPEGs. The `--extract-screenshots` path uses the same `ChangeDetector` and
/// JPEG encoding against decoded `.mov` frames.
///
/// `@unchecked Sendable`: the callback fires on a background queue; all mutable
/// state (including the `ChangeDetector`) is guarded by `lock`.
final class Screenshotter: NSObject, SCStreamOutput, @unchecked Sendable {
    let intervalMs: UInt64
    let threshold: Float
    let queue = DispatchQueue(label: "ar.tzk.panops.capture.screen")

    /// Optional tap fed every raw `.screen` `CMSampleBuffer` this screenshotter
    /// receives, BEFORE the dedup/interval gate — lets the video recorder reuse
    /// the SAME frames without a second SCStream output. Set once before
    /// capture starts; read only on `queue`.
    var videoTap: ((CMSampleBuffer) -> Void)?

    private let dir: URL
    private let lock = NSLock()
    private let ciContext = CIContext()
    private let detector: ChangeDetector
    private var startedAtMs: UInt64 = 0
    private var kept: [String] = []

    init(dir: String, intervalMs: UInt64, threshold: Float) throws {
        self.dir = URL(fileURLWithPath: dir)
        self.intervalMs = max(intervalMs, 1)
        self.threshold = threshold
        self.detector = ChangeDetector(intervalMs: intervalMs, threshold: threshold)
        super.init()
        do {
            try FileManager.default.createDirectory(at: self.dir, withIntermediateDirectories: true)
        } catch {
            throw CaptureFailure.invalidParams("could not create screenshots_dir: \(error.localizedDescription)")
        }
    }

    func markStarted(atMs ms: UInt64) {
        lock.lock(); defer { lock.unlock() }
        startedAtMs = ms
    }

    /// Paths of the JPEGs kept so far (read after `capture.stop`).
    func keptPaths() -> [String] {
        lock.lock(); defer { lock.unlock() }
        return kept
    }

    // MARK: - SCStreamOutput

    func stream(
        _ stream: SCStream,
        didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of type: SCStreamOutputType
    ) {
        guard type == .screen, CMSampleBufferDataIsReady(sampleBuffer) else { return }
        guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }

        // Forward to the video recorder (if any) BEFORE the interval/dedup gate
        // so it receives every frame; screenshot cadence below is independent.
        videoTap?(sampleBuffer)

        lock.lock()
        defer { lock.unlock() }

        let nowMs = UInt64(Date().timeIntervalSince1970 * 1000)
        guard detector.shouldKeep(atSampleMs: nowMs, featurePrint: {
            featurePrint(cvPixelBuffer: pixelBuffer)
        }) else { return }

        let tsMs = startedAtMs == 0 ? 0 : nowMs - startedAtMs
        let url = dir.appendingPathComponent(String(format: "%09d.jpg", tsMs))
        if writeJPEG(pixelBuffer, to: url) {
            kept.append(url.path)
        }
    }

    // MARK: - JPEG

    private func writeJPEG(_ pixelBuffer: CVPixelBuffer, to url: URL) -> Bool {
        let image = CIImage(cvImageBuffer: pixelBuffer)
        guard let jpeg = encodeScreenshotJPEG(image, using: ciContext) else { return false }
        do {
            try jpeg.write(to: url)
            return true
        } catch {
            FileHandle.standardError.write(Data("jpeg write failed: \(error)\n".utf8))
            return false
        }
    }
}
