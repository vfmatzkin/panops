import SwiftUI

/// The meeting-row context menu (Phase B): assign to a Space/Project, add or
/// remove Tags, and the existing Delete. Each action calls the matching IPC and
/// the view model refreshes the list. Kept as its own small view so the menu
/// tree stays out of `MeetingListView`'s row body.
struct MeetingContextMenu: View {
    @ObservedObject var vm: AppViewModel
    let meeting: MeetingSummary
    let status: MeetingStatus

    var body: some View {
        moveToSpaceMenu
        moveToProjectMenu
        addTagMenu
        removeTagMenu
        Divider()
        Button(role: .destructive) {
            Task { await vm.deleteMeeting(id: meeting.id) }
        } label: {
            Label("Delete", systemImage: "trash")
        }
        // Don't delete a meeting whose capture or notes work is still in flight.
        .disabled(!status.isDeletable)
    }

    private var moveToSpaceMenu: some View {
        Menu("Move to Space") {
            Button("Inbox") { assign(spaceId: nil, projectId: nil) }
            if !vm.spaces.isEmpty {
                Divider()
                ForEach(vm.spaces) { space in
                    Button(space.name) { assign(spaceId: space.id, projectId: nil) }
                }
            }
        }
    }

    @ViewBuilder
    private var moveToProjectMenu: some View {
        Menu("Move to Project") {
            if spacesWithProjects.isEmpty {
                Text("No projects")
            } else {
                ForEach(spacesWithProjects) { space in
                    Menu(space.name) {
                        ForEach(vm.projects(in: space.id)) { project in
                            // Assigning a project sets the space engine-side.
                            Button(project.name) { assign(spaceId: nil, projectId: project.id) }
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var addTagMenu: some View {
        Menu("Add Tag") {
            if availableTags.isEmpty {
                Text("No tags")
            } else {
                ForEach(availableTags) { tag in
                    Button(tag.name) {
                        Task { await vm.addTag(meetingId: meeting.id, tagId: tag.id) }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var removeTagMenu: some View {
        Menu("Remove Tag") {
            if assignedTags.isEmpty {
                Text("No tags")
            } else {
                ForEach(assignedTags) { tag in
                    Button(tag.name) {
                        Task { await vm.removeTag(meetingId: meeting.id, tagId: tag.id) }
                    }
                }
            }
        }
        .disabled(assignedTags.isEmpty)
    }

    // MARK: - Derived

    private var spacesWithProjects: [Space] {
        vm.spaces.filter { !vm.projects(in: $0.id).isEmpty }
    }

    private var assignedTags: [Tag] {
        let ids = Set(meeting.tags)
        return vm.tags.filter { ids.contains($0.id) }
    }

    private var availableTags: [Tag] {
        let ids = Set(meeting.tags)
        return vm.tags.filter { !ids.contains($0.id) }
    }

    private func assign(spaceId: String?, projectId: String?) {
        Task { await vm.assignMeeting(meetingId: meeting.id, spaceId: spaceId, projectId: projectId) }
    }
}
