import Foundation
import Testing
@testable import Panops

@Suite("rmsDbFS")
struct RmsDbFSTests {
    @Test("a full-scale sine sits near -3 dBFS")
    func fullScaleSine() {
        let n = 1024
        let samples = (0..<n).map { Float(sin(2 * .pi * 5 * Double($0) / Double(n))) }
        #expect(abs(rmsDbFS(samples) - (-3.0)) < 0.5)
    }

    @Test("silence clamps to the floor")
    func silenceFloor() {
        #expect(rmsDbFS([Float](repeating: 0, count: 256)) == -120)
    }

    @Test("an empty buffer reads as silence")
    func emptyIsSilence() {
        #expect(rmsDbFS([]) == -120)
    }
}

@Suite("meterFraction")
struct MeterFractionTests {
    @Test("dB maps linearly to a [0,1] fill over [-60,0]")
    func dbToFraction() {
        #expect(abs(meterFraction(db: 0) - 1.0) < 0.001)
        #expect(abs(meterFraction(db: -60) - 0.0) < 0.001)
        #expect(abs(meterFraction(db: -30) - 0.5) < 0.001)
    }

    @Test("levels past the ends clamp")
    func clamps() {
        #expect(meterFraction(db: 6) == 1.0)
        #expect(meterFraction(db: -120) == 0.0)
    }
}
