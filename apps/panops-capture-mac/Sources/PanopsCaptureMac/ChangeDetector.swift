import CoreGraphics
import CoreImage
import CoreVideo
import Foundation
@preconcurrency import Vision

// The screenshot change-detection unit, shared by the live `Screenshotter`
// (samples SCStream frames) and the `--extract-screenshots` path (decodes
// `.mov` frames). Both feed frames through the SAME interval + cosine-distance
// gate so video-derived screenshots match what the live recorder would keep.

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

/// Vision feature print of a live `.screen` pixel buffer (the live path's
/// frame source). nil on failure (logged to stderr) ⇒ the frame is skipped.
func featurePrint(cvPixelBuffer pixelBuffer: CVPixelBuffer) -> [Float]? {
    featurePrint(handler: VNImageRequestHandler(cvPixelBuffer: pixelBuffer, options: [:]))
}

/// Vision feature print of a decoded video frame (the extract path's frame
/// source). nil on failure (logged to stderr) ⇒ the frame is skipped.
func featurePrint(cgImage: CGImage) -> [Float]? {
    featurePrint(handler: VNImageRequestHandler(cgImage: cgImage, options: [:]))
}

private func featurePrint(handler: VNImageRequestHandler) -> [Float]? {
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

/// Encode a screenshot frame as JPEG, identical to the live `Screenshotter`'s
/// output (device-RGB, default quality), so video-extracted screenshots share
/// the live ones' format. Both paths route their frame through here.
func encodeScreenshotJPEG(_ image: CIImage, using ciContext: CIContext) -> Data? {
    ciContext.jpegRepresentation(of: image, colorSpace: CGColorSpaceCreateDeviceRGB(), options: [:])
}

/// Stateful keep/drop gate shared by the live `Screenshotter` and the
/// `--extract-screenshots` path. A frame is kept when it arrives at least
/// `intervalMs` after the last sampled frame AND its feature print differs from
/// the last *kept* frame by at least `threshold` cosine distance. The first
/// frame is always kept (no prior reference).
///
/// Not internally synchronized: the live `Screenshotter` calls it under its own
/// `lock`; the extract path calls it serially on one task. One instance per
/// capture/extract session.
final class ChangeDetector {
    let intervalMs: UInt64
    let threshold: Float

    private var lastSampleMs: UInt64?
    private var lastKeptVector: [Float]?

    init(intervalMs: UInt64, threshold: Float) {
        self.intervalMs = max(intervalMs, 1)
        self.threshold = threshold
    }

    /// Decide whether to keep the frame sampled at `atSampleMs`. `featurePrint`
    /// is evaluated lazily — only after the interval gate passes — so the
    /// costly Vision call is skipped for interval-dropped frames, exactly as the
    /// live path did inline. Returns true ⇒ keep the frame and adopt it as the
    /// new reference for subsequent change comparisons.
    func shouldKeep(atSampleMs nowMs: UInt64, featurePrint: () -> [Float]?) -> Bool {
        // Interval gate: drop frames arriving within `intervalMs` of the last
        // sampled one. The `nowMs >= last` guard keeps the subtraction
        // underflow-safe for any non-monotonic sample times; live wall-clock is
        // monotonic, so the guard never fires there and behavior is identical.
        if let last = lastSampleMs, nowMs >= last, nowMs - last < intervalMs { return false }
        lastSampleMs = nowMs

        guard let vector = featurePrint() else { return false }
        if let prev = lastKeptVector, cosineDistance(vector, prev) < threshold {
            return false   // near-duplicate of the last kept frame
        }
        lastKeptVector = vector
        return true
    }
}
