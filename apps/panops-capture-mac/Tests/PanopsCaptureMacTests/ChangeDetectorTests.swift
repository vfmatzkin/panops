import Foundation
import Testing
@testable import PanopsCaptureMac

/// Covers the stateful keep/drop gate the live `Screenshotter` and the
/// `--extract-screenshots` path share: feeds it feature-print vectors at chosen
/// sample times and asserts it keeps a frame when cosine distance exceeds the
/// threshold and skips when below, plus the interval gate. The pure cosine math
/// is exercised separately in `DedupTests`.
struct ChangeDetectorTests {
    // Reference vectors: `a` vs `b` are orthogonal (distance 1 ≥ 0.15 ⇒ keep);
    // `aNear` is a tiny perturbation of `a` (distance < 0.15 ⇒ drop).
    private let a: [Float] = [1, 0, 0]
    private let b: [Float] = [0, 1, 0]
    private let aNear: [Float] = [0.99, 0.01, 0]

    @Test func firstFrameIsAlwaysKept() {
        let d = ChangeDetector(intervalMs: 100, threshold: 0.15)
        #expect(d.shouldKeep(atSampleMs: 0) { self.a })
    }

    @Test func keepsWhenCosineDistanceExceedsThreshold() {
        let d = ChangeDetector(intervalMs: 100, threshold: 0.15)
        #expect(d.shouldKeep(atSampleMs: 0) { self.a })       // kept (first)
        #expect(d.shouldKeep(atSampleMs: 200) { self.b })     // orthogonal → kept
    }

    @Test func skipsWhenCosineDistanceBelowThreshold() {
        let d = ChangeDetector(intervalMs: 100, threshold: 0.15)
        #expect(d.shouldKeep(atSampleMs: 0) { self.a })       // kept (first)
        #expect(!d.shouldKeep(atSampleMs: 200) { self.aNear }) // near-dup → dropped
    }

    @Test func comparesAgainstLastKeptNotLastSeen() {
        let d = ChangeDetector(intervalMs: 100, threshold: 0.15)
        #expect(d.shouldKeep(atSampleMs: 0) { self.a })        // keep `a` (reference)
        #expect(!d.shouldKeep(atSampleMs: 200) { self.aNear }) // dropped; reference stays `a`
        #expect(!d.shouldKeep(atSampleMs: 400) { self.aNear }) // still near `a` → dropped
        #expect(d.shouldKeep(atSampleMs: 600) { self.b })      // far from `a` → kept
    }

    @Test func intervalGateDropsFramesWithinInterval() {
        let d = ChangeDetector(intervalMs: 500, threshold: 0.15)
        #expect(d.shouldKeep(atSampleMs: 0) { self.a })        // kept
        // 100ms later, totally different content, still dropped by the interval.
        #expect(!d.shouldKeep(atSampleMs: 100) { self.b })
        // At the interval boundary the frame passes the interval gate again.
        #expect(d.shouldKeep(atSampleMs: 500) { self.b })
    }

    @Test func intervalDroppedFrameSkipsTheFeaturePrintWork() {
        let d = ChangeDetector(intervalMs: 500, threshold: 0.15)
        #expect(d.shouldKeep(atSampleMs: 0) { self.a })
        var evaluated = false
        _ = d.shouldKeep(atSampleMs: 100) { evaluated = true; return self.b }
        #expect(!evaluated)   // interval-dropped ⇒ Vision feature print never computed
    }

    @Test func nilFeaturePrintIsNotKept() {
        let d = ChangeDetector(intervalMs: 100, threshold: 0.15)
        #expect(!d.shouldKeep(atSampleMs: 0) { nil })
    }
}
