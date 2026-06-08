import SwiftUI

/// A horizontal dBFS level meter. Fills left-to-right from `meterFraction(db:)`,
/// greening through yellow into red near clipping. Labelled with an SF Symbol +
/// caption so a glance tells you which source it is and that it's moving.
struct LevelMeter: View {
    let label: String
    let systemImage: String
    let db: Float

    private var fraction: Double { meterFraction(db: db) }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: systemImage)
                .frame(width: 18)
                .foregroundStyle(.secondary)
            bar
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 72, alignment: .leading)
        }
    }

    private var bar: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.secondary.opacity(0.18))
                Capsule()
                    .fill(fillColor)
                    .frame(width: max(0, geo.size.width * fraction))
            }
        }
        .frame(height: 8)
        .animation(.linear(duration: 0.08), value: fraction)
    }

    private var fillColor: Color {
        if fraction > 0.85 { return .red }
        if fraction > 0.6 { return .yellow }
        return .green
    }
}
