import Foundation
import SwiftUI
import UniformTypeIdentifiers

/// Payload carried by a meeting-row drag. Exposes the meeting id as a plain
/// String so the drop destination can read it without needing a custom UTType
/// declaration in Info.plist. The type is internal to the app; there is no
/// inter-app drag surface.
struct MeetingDragPayload: Transferable, Equatable {
    let meetingId: String

    static var transferRepresentation: some TransferRepresentation {
        ProxyRepresentation(exporting: \.meetingId)
    }
}

/// Describes the org row a meeting was dropped on. Pure data — the view builds
/// one from the sidebar row type, the view model executes it. Separated so the
/// drop-target mapping is testable without an IPC client.
enum MeetingDropTarget: Equatable {
    case space(spaceId: String)
    case project(projectId: String)
    case tag(tagId: String)

    /// Short label for the SaveStatusView failure message.
    var operationNoun: String {
        switch self {
        case .space: return "space"
        case .project: return "project"
        case .tag: return "tag"
        }
    }
}
