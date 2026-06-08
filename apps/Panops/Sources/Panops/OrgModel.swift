import Foundation

/// Built-in cross-cutting filters shown in the sidebar's "Smart Views" section.
/// These filter the loaded meeting list client-side (no dedicated IPC method);
/// space/project/tag selections refetch through the engine's `meeting.list`
/// filters instead.
enum SmartView: String, CaseIterable, Identifiable, Hashable {
    case inbox
    case all
    case needsNotes
    case thisWeek

    var id: String { rawValue }

    var title: String {
        switch self {
        case .inbox: return "Inbox"
        case .all: return "All"
        case .needsNotes: return "Needs Notes"
        case .thisWeek: return "This Week"
        }
    }

    var systemImage: String {
        switch self {
        case .inbox: return "tray"
        case .all: return "rectangle.stack"
        case .needsNotes: return "doc.badge.ellipsis"
        case .thisWeek: return "calendar"
        }
    }
}

/// The current sidebar selection that drives which meetings the content list
/// shows. The default `.smart(.all)` lists every meeting (the pre-Phase-B
/// behavior).
enum SidebarSelection: Hashable {
    case smart(SmartView)
    case space(String)    // space id
    case project(String)  // project id
    case tag(String)      // tag id
}
