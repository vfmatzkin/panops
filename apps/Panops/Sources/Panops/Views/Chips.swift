import SwiftUI

/// Lifecycle status of a meeting, derived in `AppViewModel.status(for:)`.
enum MeetingStatus: Equatable {
    case recording
    case processing
    case ready
    case needsNotes
    case draft

    var label: String {
        switch self {
        case .recording: return "Recording"
        case .processing: return "Processing"
        case .ready: return "Ready"
        case .needsNotes: return "Needs notes"
        case .draft: return "Draft"
        }
    }

    var color: Color {
        switch self {
        case .recording: return .red
        case .processing: return .orange
        case .ready: return .green
        case .needsNotes: return .blue
        case .draft: return .secondary
        }
    }

    var systemImage: String {
        switch self {
        case .recording: return "record.circle"
        case .processing: return "gearshape"
        case .ready: return "checkmark.circle"
        case .needsNotes: return "doc.badge.ellipsis"
        case .draft: return "circle.dashed"
        }
    }

    /// Whether deleting this meeting is safe. Recording and processing meetings
    /// have active capture / notes work that owns the row and directory, so
    /// removing them out from under that work is blocked.
    var isDeletable: Bool {
        switch self {
        case .recording, .processing: return false
        case .ready, .needsNotes, .draft: return true
        }
    }
}

/// Small colored status pill for sidebar rows.
struct StatusPill: View {
    let status: MeetingStatus

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: status.systemImage)
                .imageScale(.small)
            Text(status.label)
        }
        .font(.caption2.weight(.medium))
        .padding(.horizontal, 7)
        .padding(.vertical, 2)
        .background(status.color.opacity(0.15))
        .foregroundStyle(status.color)
        .clipShape(Capsule())
    }
}

/// A trust/metadata chip: icon + label in a subtle pill. Used in the meeting
/// header to convey on-device, private processing honestly.
struct TrustChip: View {
    let systemImage: String
    let label: String
    var tint: Color = .secondary

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: systemImage).imageScale(.small)
            Text(label)
        }
        .font(.caption)
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(tint.opacity(0.12))
        .foregroundStyle(tint)
        .clipShape(Capsule())
    }
}

/// The honest local-first trust strip: "Local · Private · your data stays on
/// this Mac".
struct TrustStrip: View {
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "lock.shield")
            Text("Local · Private · your data stays on this Mac")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}
