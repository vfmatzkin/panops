import Foundation

/// RMS of `samples` (nominal [-1, 1]) in dBFS, floored at -120 for silence.
/// Pure so the level math is unit-testable off any audio device.
func rmsDbFS(_ samples: [Float]) -> Float {
    guard !samples.isEmpty else { return -120 }
    let meanSquare = samples.reduce(Float(0)) { $0 + $1 * $1 } / Float(samples.count)
    guard meanSquare > 0 else { return -120 }
    return max(-120, 20 * log10(sqrt(meanSquare)))
}

/// Map a dBFS level to a [0, 1] meter fill over the useful range [-60, 0]:
/// 0 dBFS fills the bar, -60 dBFS (or quieter) empties it, linear in between.
func meterFraction(db: Float) -> Double {
    Double(max(0, min(1, (db + 60) / 60)))
}
