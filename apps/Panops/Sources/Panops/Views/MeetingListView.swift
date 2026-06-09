import SwiftUI

/// Sidebar: meetings grouped by day (Today / Yesterday / dated), with rich rows
/// (title, time, duration, language badge, status pill), a search field, and a
/// context-menu delete. Selection drives the detail workspace.
struct MeetingListView: View {
    @ObservedObject var vm: AppViewModel
    @State private var searchText = ""

    var body: some View {
        VStack(spacing: 0) {
            searchBar
            Divider()
            content
        }
        .toolbar {
            ToolbarItem {
                Button {
                    Task { await vm.refreshMeetings() }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .help("Refresh meetings")
            }
        }
        .task {
            await vm.refreshMeetings()
        }
    }

    private var searchBar: some View {
        SearchField(placeholder: "Search meetings", text: $searchText)
            .padding(8)
    }

    @ViewBuilder
    private var content: some View {
        if vm.meetings.isEmpty {
            placeholder("No meetings yet")
        } else {
            let sections = groupedSections
            if sections.isEmpty {
                placeholder("No matches")
            } else {
                List(selection: $vm.selectedMeetingId) {
                    ForEach(sections) { section in
                        Section(section.title) {
                            ForEach(section.meetings, id: \.id) { meeting in
                                let status = vm.status(for: meeting)
                                MeetingRow(
                                    meeting: meeting,
                                    status: status,
                                    tagNames: meeting.tags.map { vm.tagName($0) }
                                )
                                .tag(meeting.id)
                                .draggable(MeetingDragPayload(meetingId: meeting.id))
                                .contextMenu {
                                    MeetingContextMenu(vm: vm, meeting: meeting, status: status)
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    private func placeholder(_ text: String) -> some View {
        VStack {
            Spacer()
            Text(text).foregroundStyle(.secondary)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Filtering / grouping

    private var filteredMeetings: [MeetingSummary] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else { return vm.meetings }
        return vm.meetings.filter { meeting in
            if meeting.title.lowercased().contains(query) { return true }
            if meeting.language.lowercased().contains(query) { return true }
            if meeting.startedAt.lowercased().contains(query) { return true }
            if let date = MeetingDate.parse(meeting.startedAt),
               MeetingDate.shortDate(date).lowercased().contains(query) { return true }
            return false
        }
    }

    private struct MeetingSection: Identifiable {
        let id: String
        let title: String
        let sortKey: Date
        let meetings: [MeetingSummary]
    }

    private var groupedSections: [MeetingSection] {
        let calendar = Calendar.current
        var buckets: [Date: [MeetingSummary]] = [:]
        var undated: [MeetingSummary] = []

        for meeting in filteredMeetings {
            if let date = MeetingDate.parse(meeting.startedAt) {
                buckets[calendar.startOfDay(for: date), default: []].append(meeting)
            } else {
                undated.append(meeting)
            }
        }

        func sortedByStartDescending(_ items: [MeetingSummary]) -> [MeetingSummary] {
            items.sorted { lhs, rhs in
                let l = MeetingDate.parse(lhs.startedAt) ?? .distantPast
                let r = MeetingDate.parse(rhs.startedAt) ?? .distantPast
                return l > r
            }
        }

        var sections = buckets.map { day, items in
            MeetingSection(
                id: ISO8601DateFormatter().string(from: day),
                title: MeetingDate.dayLabel(day),
                sortKey: day,
                meetings: sortedByStartDescending(items)
            )
        }
        sections.sort { $0.sortKey > $1.sortKey }

        if !undated.isEmpty {
            sections.append(
                MeetingSection(
                    id: "undated",
                    title: "Earlier",
                    sortKey: .distantPast,
                    meetings: undated
                )
            )
        }
        return sections
    }
}

/// A rich sidebar row: title + status pill, then time · duration · language,
/// then tag chips (Phase B) when the meeting carries any.
struct MeetingRow: View {
    let meeting: MeetingSummary
    let status: MeetingStatus
    var tagNames: [String] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(meeting.title.isEmpty ? "Untitled meeting" : meeting.title)
                    .font(.headline)
                    .lineLimit(1)
                Spacer(minLength: 8)
                StatusPill(status: status)
            }
            HStack(spacing: 6) {
                Text(metaLine)
                if !meeting.language.isEmpty {
                    Text(meeting.language.uppercased())
                        .font(.caption2.weight(.medium))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Color.secondary.opacity(0.15))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            if !tagNames.isEmpty {
                tagChips
            }
        }
        .padding(.vertical, 2)
    }

    private var tagChips: some View {
        HStack(spacing: 4) {
            Image(systemName: "tag")
                .imageScale(.small)
                .foregroundStyle(.secondary)
            ForEach(Array(tagNames.prefix(4)), id: \.self) { name in
                Text(name)
                    .font(.caption2)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(Color.accentColor.opacity(0.15))
                    .clipShape(Capsule())
            }
            if tagNames.count > 4 {
                Text("+\(tagNames.count - 4)")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var metaLine: String {
        var parts: [String] = []
        if let date = MeetingDate.parse(meeting.startedAt) {
            parts.append(MeetingDate.shortTime(date))
        }
        if meeting.durationMs > 0 {
            parts.append(MeetingDate.duration(ms: meeting.durationMs))
        }
        return parts.joined(separator: " · ")
    }
}
