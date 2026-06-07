import Foundation
import Testing
@testable import PanopsCaptureMac

/// Covers the pure screenshot-dedup math: cosine distance vs the default
/// `0.15` threshold decides keep/drop. The Vision feature-print extraction and
/// JPEG write are exercised by the manual Mac smoke.
struct DedupTests {
    private let threshold: Float = 0.15

    @Test func identicalVectorsAreDropped() {
        let a: [Float] = [1, 0, 0]
        let b: [Float] = [1, 0, 0]
        #expect(cosineDistance(a, b) < threshold)        // near-dup → drop
    }

    @Test func orthogonalVectorsAreKept() {
        let a: [Float] = [1, 0, 0]
        let b: [Float] = [0, 1, 0]
        #expect(cosineDistance(a, b) >= threshold)       // distinct → keep
    }

    @Test func scaledParallelVectorsAreDropped() {
        // Same direction, different magnitude → cosine distance 0.
        let a: [Float] = [2, 4, 6]
        let b: [Float] = [1, 2, 3]
        #expect(abs(cosineDistance(a, b)) < 1e-5)
    }

    @Test func nearlyIdenticalStaysBelowThreshold() {
        let a: [Float] = [1.0, 0.0, 0.0]
        let b: [Float] = [0.99, 0.01, 0.0]
        #expect(cosineDistance(a, b) < threshold)        // tiny change → still a dup
    }

    @Test func oppositeVectorsAreMaxDistance() {
        let a: [Float] = [1, 0, 0]
        let b: [Float] = [-1, 0, 0]
        #expect(abs(cosineDistance(a, b) - 2) < 1e-5)
    }

    @Test func mismatchedLengthsConservativelyKeep() {
        #expect(cosineDistance([1, 0], [1, 0, 0]) == 1)
        #expect(cosineDistance([], []) == 1)
    }

    @Test func zeroVectorConservativelyKeeps() {
        #expect(cosineDistance([0, 0, 0], [1, 1, 1]) == 1)
    }
}
