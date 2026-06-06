import SwiftUI

/// Sidebar showing meetings from meeting.list.
/// Slice 12 implementation per spec D1.
struct MeetingListView: View {
    @ObservedObject var vm: AppViewModel

    var body: some View {
        List(selection: $vm.selectedMeetingId) {
            if vm.meetings.isEmpty {
                Text("No meetings yet").foregroundStyle(.secondary)
            } else {
                ForEach(vm.meetings, id: \.id) { meeting in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(meeting.title).font(.headline)
                        Text(meeting.startedAt).font(.caption).foregroundStyle(.secondary)
                    }
                    .tag(meeting.id)
                }
            }
        }
        .toolbar {
            ToolbarItem {
                Button("Refresh") {
                    Task { await vm.refreshMeetings() }
                }
            }
        }
        .task {
            await vm.refreshMeetings()
        }
    }
}