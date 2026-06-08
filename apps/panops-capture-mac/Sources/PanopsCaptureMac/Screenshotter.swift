import CoreImage
import CoreMedia
import Foundation
import ScreenCaptureKit
import Vision

/// Cosine *distance* (1 − cosine similarity) between two feature vectors.
/// Range [0, 2]; 0 = identical direction. Pure, so dedup math is testable
/// without Vision. Mismatched lengths or a zero vector → maximal distance (1)
/// so the frame is conservatively kept.
func cosineDistance(_ a: [Float], _ b: [Float]) -> Float {
    guard a.count == b.count, !a.isEmpty else { return 1 }
    let dot = zip(a, b).reduce(Float(0)) { $0 + $1.0 * $1.1 }
    let na = sqrt(a.reduce(Float(0)) { $0 + $1 * $1 })
    let nb = sqrt(b.reduce(Float(0)) { $0 + $1 * $1 })
    guard na > 0, nb > 0 else { return 1 }
    return 1 - dot / (na * nb)
}

/// Extract the Float32 vector backing a Vision feature print.
func featurePrintVector(_ observation: VNFeaturePrintObservation) -> [Float] {
    let count = observation.elementCount
    guard count > 0 else { return [] }
    var out = [Float](repeating: 0, count: count)
    out.withUnsafeMutableBytes { dst in
        _ = observation.data.copyBytes(to: dst, count: count * MemoryLayout<Float>.stride)
    }
    return out
}

/// Samples `SCStream` `.screen` frames, dedups near-duplicates via a Vision
/// feature print (cosine distance vs the last *kept* frame, below
/// `threshold` ⇒ drop), and writes the kept ones as time-anchored JPEGs.
///
/// `@unchecked Sendable`: the callback fires on a background queue; all mutable
/// state is guarded by `lock`.
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
    private var startedAtMs: UInt64 = 0
    private var lastSampleMs: UInt64?
    private var lastKeptVector: [Float]?
    private var kept: [String] = []

    init(dir: String, intervalMs: UInt64, threshold: Float) throws {
        self.dir = URL(fileURLWithPath: dir)
        self.intervalMs = max(intervalMs, 1)
        self.threshold = threshold
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
        if let last = lastSampleMs, nowMs - last < intervalMs { return }
        lastSampleMs = nowMs

        guard let vector = featurePrint(of: pixelBuffer) else { return }
        if let prev = lastKeptVector, cosineDistance(vector, prev) < threshold {
            return   // near-duplicate of the last kept frame
        }
        lastKeptVector = vector

        let tsMs = startedAtMs == 0 ? 0 : nowMs - startedAtMs
        let url = dir.appendingPathComponent(String(format: "%09d.jpg", tsMs))
        if writeJPEG(pixelBuffer, to: url) {
            kept.append(url.path)
        }
    }

    // MARK: - Vision + JPEG

    private func featurePrint(of pixelBuffer: CVPixelBuffer) -> [Float]? {
        let handler = VNImageRequestHandler(cvPixelBuffer: pixelBuffer, options: [:])
        let request = VNGenerateImageFeaturePrintRequest()
        do {
            try handler.perform([request])
        } catch {
            FileHandle.standardError.write(Data("feature print failed: \(error)\n".utf8))
            return nil
        }
        guard let observation = request.results?.first else { return nil }
        return featurePrintVector(observation)
    }

    private func writeJPEG(_ pixelBuffer: CVPixelBuffer, to url: URL) -> Bool {
        let image = CIImage(cvImageBuffer: pixelBuffer)
        guard let jpeg = ciContext.jpegRepresentation(
            of: image, colorSpace: CGColorSpaceCreateDeviceRGB(), options: [:]
        ) else { return false }
        do {
            try jpeg.write(to: url)
            return true
        } catch {
            FileHandle.standardError.write(Data("jpeg write failed: \(error)\n".utf8))
            return false
        }
    }
}
