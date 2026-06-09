import Foundation

/// dBFS value reported for digital silence — the floor `rmsDbFS` clamps to and
/// the level meters reset to. Shared so the meter math and the published levels
/// agree on one "silence" value.
let silenceFloorDb: Float = -120

/// Quietest level the meter renders: at or below this the bar reads empty, 0
/// dBFS fills it, linear in between. The useful display range is `[meterFloorDb, 0]`.
let meterFloorDb: Float = -60

/// RMS of `samples` (nominal [-1, 1]) in dBFS, floored at `silenceFloorDb` for
/// silence. Pure so the level math is unit-testable off any audio device.
func rmsDbFS(_ samples: [Float]) -> Float {
    guard !samples.isEmpty else { return silenceFloorDb }
    let meanSquare = samples.reduce(Float(0)) { $0 + $1 * $1 } / Float(samples.count)
    guard meanSquare > 0 else { return silenceFloorDb }
    return max(silenceFloorDb, 20 * log10(sqrt(meanSquare)))
}

/// Map a dBFS level to a [0, 1] meter fill over the useful range
/// `[meterFloorDb, 0]`: 0 dBFS fills the bar, `meterFloorDb` (or quieter) empties
/// it, linear in between.
func meterFraction(db: Float) -> Double {
    let range = -meterFloorDb
    return Double(max(0, min(1, (db - meterFloorDb) / range)))
}
