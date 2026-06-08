import SwiftUI

/// The organization sidebar (Phase B): three sections — Smart Views, Spaces
/// (collapsible → Projects), and Tags. Selecting a row drives the meeting
/// list's filter via `vm.sidebarSelection`. "+" creates; context menus
/// rename/delete. No drag-and-drop (context-menu only).
struct OrgSidebarView: View {
    @ObservedObject var vm: AppViewModel
    /// Which spaces are expanded to show their projects.
    @State private var expandedSpaces: Set<String> = []
    /// The active create/rename prompt, if any (drives the name alert).
    @State private var prompt: OrgPrompt?
    @State private var promptText: String = ""

    var body: some View {
        List(selection: selectionBinding) {
            smartViewsSection
            spacesSection
            tagsSection
        }
        .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 340)
        .alert(prompt?.title ?? "", isPresented: promptPresented) {
            TextField(prompt?.fieldPrompt ?? "Name", text: $promptText)
            Button(prompt?.actionLabel ?? "OK") { submitPrompt() }
            Button("Cancel", role: .cancel) { prompt = nil }
        }
    }

    // MARK: - Sections

    private var smartViewsSection: some View {
        Section("Smart Views") {
            ForEach(SmartView.allCases) { view in
                Label(view.title, systemImage: view.systemImage)
                    .tag(SidebarSelection.smart(view))
            }
        }
    }

    private var spacesSection: some View {
        Section {
            ForEach(vm.spaces) { space in
                spaceRow(space)
                if expandedSpaces.contains(space.id) {
                    ForEach(vm.projects(in: space.id)) { project in
                        projectRow(project)
                    }
                }
            }
        } header: {
            sectionHeader("Spaces", add: "New space") { beginPrompt(.newSpace) }
        }
    }

    private var tagsSection: some View {
        Section {
            ForEach(vm.tags) { tag in
                tagRow(tag)
            }
        } header: {
            sectionHeader("Tags", add: "New tag") { beginPrompt(.newTag) }
        }
    }

    // MARK: - Rows

    private func spaceRow(_ space: Space) -> some View {
        HStack(spacing: 4) {
            Button { toggleExpanded(space.id) } label: {
                Image(systemName: expandedSpaces.contains(space.id) ? "chevron.down" : "chevron.right")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .frame(width: 12)
            }
            .buttonStyle(.plain)
            Label(space.name, systemImage: "folder")
            Spacer(minLength: 4)
            Button { beginPrompt(.newProject(spaceId: space.id)) } label: {
                Image(systemName: "plus").imageScale(.small)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("New project in \(space.name)")
        }
        .tag(SidebarSelection.space(space.id))
        .contextMenu { spaceContextMenu(space) }
    }

    private func projectRow(_ project: Project) -> some View {
        Label(project.name, systemImage: "list.bullet")
            .padding(.leading, 16)
            .tag(SidebarSelection.project(project.id))
            .contextMenu { projectContextMenu(project) }
    }

    private func tagRow(_ tag: Tag) -> some View {
        Label(tag.name, systemImage: "tag")
            .tag(SidebarSelection.tag(tag.id))
            .contextMenu {
                Button(role: .destructive) {
                    Task { await vm.deleteTag(id: tag.id) }
                } label: {
                    Label("Delete Tag", systemImage: "trash")
                }
            }
    }

    private func sectionHeader(_ title: String, add help: String, action: @escaping () -> Void) -> some View {
        HStack {
            Text(title)
            Spacer()
            Button(action: action) {
                Image(systemName: "plus").imageScale(.small)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help(help)
        }
    }

    // MARK: - Context menus

    @ViewBuilder
    private func spaceContextMenu(_ space: Space) -> some View {
        Button { beginPrompt(.newProject(spaceId: space.id)) } label: {
            Label("New Project", systemImage: "plus")
        }
        Button { beginPrompt(.renameSpace(id: space.id, current: space.name)) } label: {
            Label("Rename", systemImage: "pencil")
        }
        Button(role: .destructive) {
            Task { await vm.deleteSpace(id: space.id) }
        } label: {
            Label("Delete Space", systemImage: "trash")
        }
    }

    @ViewBuilder
    private func projectContextMenu(_ project: Project) -> some View {
        Button { beginPrompt(.renameProject(id: project.id, current: project.name)) } label: {
            Label("Rename", systemImage: "pencil")
        }
        Button(role: .destructive) {
            Task { await vm.deleteProject(id: project.id) }
        } label: {
            Label("Delete Project", systemImage: "trash")
        }
    }

    // MARK: - Selection binding

    /// `List(selection:)` wants an optional; map it onto the view model's
    /// non-optional `sidebarSelection`, ignoring deselection (nil) so a filter
    /// is always active.
    private var selectionBinding: Binding<SidebarSelection?> {
        Binding(
            get: { vm.sidebarSelection },
            set: { newValue in
                if let newValue { vm.sidebarSelection = newValue }
            }
        )
    }

    // MARK: - Expansion + prompts

    private func toggleExpanded(_ spaceId: String) {
        if expandedSpaces.contains(spaceId) {
            expandedSpaces.remove(spaceId)
        } else {
            expandedSpaces.insert(spaceId)
        }
    }

    private var promptPresented: Binding<Bool> {
        Binding(
            get: { prompt != nil },
            set: { if !$0 { prompt = nil } }
        )
    }

    private func beginPrompt(_ p: OrgPrompt) {
        switch p {
        case .renameSpace(_, let current), .renameProject(_, let current):
            promptText = current
        case .newSpace, .newProject, .newTag:
            promptText = ""
        }
        prompt = p
    }

    private func submitPrompt() {
        guard let prompt else { return }
        let text = promptText
        Task {
            switch prompt {
            case .newSpace:
                await vm.createSpace(name: text)
            case .newProject(let spaceId):
                await vm.createProject(spaceId: spaceId, name: text)
            case .renameSpace(let id, _):
                await vm.renameSpace(id: id, name: text)
            case .renameProject(let id, _):
                await vm.renameProject(id: id, name: text)
            case .newTag:
                await vm.createTag(name: text)
            }
        }
        self.prompt = nil
    }
}

/// A pending create/rename action that the name alert collects text for.
enum OrgPrompt: Identifiable {
    case newSpace
    case newProject(spaceId: String)
    case renameSpace(id: String, current: String)
    case renameProject(id: String, current: String)
    case newTag

    var id: String {
        switch self {
        case .newSpace: return "newSpace"
        case .newProject(let spaceId): return "newProject-\(spaceId)"
        case .renameSpace(let id, _): return "renameSpace-\(id)"
        case .renameProject(let id, _): return "renameProject-\(id)"
        case .newTag: return "newTag"
        }
    }

    var title: String {
        switch self {
        case .newSpace: return "New Space"
        case .newProject: return "New Project"
        case .renameSpace: return "Rename Space"
        case .renameProject: return "Rename Project"
        case .newTag: return "New Tag"
        }
    }

    var actionLabel: String {
        switch self {
        case .newSpace, .newProject, .newTag: return "Create"
        case .renameSpace, .renameProject: return "Rename"
        }
    }

    var fieldPrompt: String {
        switch self {
        case .newTag: return "Tag name"
        default: return "Name"
        }
    }
}
