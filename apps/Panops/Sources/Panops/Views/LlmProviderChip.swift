import SwiftUI

struct LlmProviderChip: View {
    let info: LlmInfo

    private var label: String {
        if info.local {
            return "Local · \(info.provider)/\(info.model)"
        }
        return "⚠︎ Cloud · \(info.provider)/\(info.model)"
    }

    private var tint: Color {
        info.local ? Color.secondary : Color.orange
    }

    private var fill: Color {
        info.local ? Color.secondary.opacity(0.12) : Color.orange.opacity(0.15)
    }

    var body: some View {
        Text(label)
            .font(.caption)
            .lineLimit(1)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .foregroundStyle(tint)
            .background(Capsule().fill(fill))
            .overlay(Capsule().stroke(tint.opacity(0.35), lineWidth: 1))
            .help(label)
    }
}
