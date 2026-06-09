import SwiftUI

/// Renders meeting notes as a clean document.
///
/// Preferred path: a decoded `StructuredNotes` IR (`notes.json`) — rendered as
/// sections with narrative, key points, action items, screenshots, and a tag
/// row. No raw markdown, no YAML.
///
/// Fallback: a `notes.md` string — the leading YAML frontmatter is stripped and
/// the body is rendered with block structure (headings become styled titles).
///
/// Neither present → placeholder.
///
/// ## Editing
///
/// When `onSave` is supplied, a small Edit toggle appears in the top-right.
/// Turning it on swaps the rendered view for a markdown `TextEditor` bound to
/// the raw `notes.md` source. Turning it off — or the view being torn down
/// mid-edit (a meeting switch or notes regeneration both reset NotesView's
/// identity) — autosaves the buffer via `onSave`; on success the view
/// re-renders from the saved markdown (structured IR is bypassed until the next
/// regeneration, which writes fresh `notes.json` + `notes.md`). On failure the
/// edited text stays in the editor and the SaveStatus chip surfaces Retry —
/// edits are never silently lost.
struct NotesView: View {
    let notes: StructuredNotes?
    let markdownFallback: String?
    let meetingDir: String
    /// Autosave hook. `nil` disables editing (e.g. previews, no-meeting
    /// states). Called on toggle-out of edit mode; returns success/failure.
    /// Marked `@Sendable` so the closure can be captured by the MainActor
    /// Task that performs the IPC under Swift 6 concurrency.
    var onSave: (@Sendable (String) async -> Bool)? = nil

    @State private var editing = false
    /// Working buffer for the markdown editor. Loaded lazily on first entry
    /// into edit mode from the current rendered markdown; edited in place.
    @State private var editorBuffer: String = ""
    /// After a successful save, the saved markdown is authoritative. The
    /// rendered view uses this instead of `notes` / `markdownFallback` so a
    /// manual edit is reflected immediately, even when `notes.json` still
    /// exists on disk (the structured IR becomes stale until regeneration).
    @State private var overriddenMarkdown: String? = nil
    /// True once the editor buffer has been populated from an edit session.
    /// Guards `commitIfDirty` so a teardown or toggle never persists the empty
    /// initial buffer over real notes when the user never entered edit mode.
    @State private var bufferLoaded = false
    /// True while an autosave is in flight, so an edit-toggle racing a teardown
    /// can't spawn two concurrent saves of the same buffer.
    @State private var isSaving = false

    init(
        notes: StructuredNotes?,
        markdownFallback: String?,
        meetingDir: String = "",
        onSave: (@Sendable (String) async -> Bool)? = nil
    ) {
        self.notes = notes
        self.markdownFallback = markdownFallback
        self.meetingDir = meetingDir
        self.onSave = onSave
    }

    /// Convenience for markdown-only callers (legacy `NotesView(content:)`).
    init(content: String?) {
        self.notes = nil
        self.markdownFallback = content
        self.meetingDir = ""
        self.onSave = nil
    }

    /// Markdown that should be rendered right now. Post-save, the edited
    /// buffer wins; otherwise the parent's loaded markdown.
    private var renderedMarkdown: String? {
        overriddenMarkdown ?? markdownFallback
    }

    /// Structured notes to render. Suppressed once the user has manually
    /// edited the markdown (the edit is authoritative until regeneration).
    private var renderedStructured: StructuredNotes? {
        overriddenMarkdown != nil ? nil : notes
    }

    private var hasNotesToDisplay: Bool {
        if renderedStructured != nil { return true }
        if let md = renderedMarkdown, !md.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return true
        }
        return false
    }

    var body: some View {
        VStack(spacing: 0) {
            if hasNotesToDisplay, onSave != nil {
                editToggleBar
            }

            if editing {
                notesEditor
            } else if let renderedStructured {
                structuredDocument(renderedStructured)
            } else if let md = renderedMarkdown, !md.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                ScrollView {
                    MarkdownBlocksView(markdown: Markdown.stripFrontmatter(md))
                        .padding(20)
                }
            } else {
                placeholder
            }
        }
        .onChange(of: editing) { wasEditing, isEditing in
            if isEditing {
                // Lazy load: the buffer is populated from the current
                // rendered markdown on first entry into edit mode. Stays
                // intact across subsequent toggle flaps within the same
                // meeting so the user's in-progress text isn't lost if
                // they flip Edit off and back on before saving.
                if !bufferLoaded {
                    editorBuffer = renderedMarkdown ?? ""
                    bufferLoaded = true
                }
            } else if wasEditing {
                commitIfDirty()
            }
        }
        // Persist on teardown. NotesView is identity-bound to the meeting +
        // notesReloadTick in MeetingDetailView, so switching meetings or a
        // notes regeneration tears this view down *without* `editing` toggling
        // off — `commitIfDirty` would otherwise never run and an in-progress
        // edit would be silently discarded.
        .onDisappear { commitIfDirty() }
    }

    // MARK: - Edit toggle bar

    private var editToggleBar: some View {
        HStack {
            Spacer()
            Toggle(isOn: $editing) {
                Label(
                    editing ? "Done" : "Edit",
                    systemImage: editing ? "checkmark.circle" : "pencil"
                )
            }
            .toggleStyle(.button)
            .controlSize(.small)
            .help(editing ? "Finish editing and autosave" : "Edit notes as Markdown")
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
    }

    // MARK: - Editor

    private var notesEditor: some View {
        TextEditor(text: $editorBuffer)
            .font(.system(.body, design: .monospaced))
            .scrollContentBackground(.hidden)
            .padding(16)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Save

    /// Persist the buffer via the parent's `onSave`. No-op when the user never
    /// entered edit mode, when the buffer equals the current rendered markdown
    /// (nothing to save), or when a save is already in flight. On failure,
    /// re-enters edit mode so the user can Retry; on success, swaps the
    /// rendered view to the saved markdown.
    private func commitIfDirty() {
        // Only persist content the user actually edited. Without this a
        // teardown before the user ever entered edit mode would save the empty
        // initial buffer over the real notes.
        guard bufferLoaded else { return }
        guard !isSaving else { return }
        let current = renderedMarkdown ?? ""
        guard editorBuffer != current else { return }
        guard let onSave else { return }
        let buffer = editorBuffer
        isSaving = true
        Task {
            let ok = await onSave(buffer)
            isSaving = false
            if ok {
                overriddenMarkdown = buffer
            } else {
                // Restore edit mode with the buffer intact so the user can
                // Retry via the SaveStatus chip in the header.
                editing = true
            }
        }
    }

    // MARK: - Rendering

    @ViewBuilder
    private func structuredDocument(_ notes: StructuredNotes) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                ForEach(notes.sections) { section in
                    sectionView(section)
                }
                if !notes.frontmatter.tags.isEmpty {
                    tagRow(notes.frontmatter.tags)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private func sectionView(_ section: NotesSection) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            if isMeaningfulTitle(section.title) {
                Text(section.title)
                    .font(.title3)
                    .fontWeight(.bold)
            }

            if !section.narrativeMd.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                MarkdownBlocksView(markdown: section.narrativeMd)
            }

            if !section.keyPoints.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Label("Key points", systemImage: "star")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ForEach(Array(section.keyPoints.enumerated()), id: \.offset) { _, point in
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text("•").foregroundStyle(.secondary)
                            Text(Markdown.inlineAttributed(point))
                                .textSelection(.enabled)
                        }
                    }
                }
            }

            if !section.actionItems.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Label("Action items", systemImage: "checklist")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ForEach(section.actionItems) { item in
                        ActionItemRow(item: item)
                    }
                }
            }

            let shots = resolvedScreenshots(section.screenshots)
            if !shots.isEmpty {
                ScreenshotsStripView(urls: shots)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func tagRow(_ tags: [String]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Divider()
            FlowLayout(spacing: 8, lineSpacing: 8) {
                ForEach(Array(tags.enumerated()), id: \.offset) { _, tag in
                    Text(tag)
                        .font(.caption)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 4)
                        .background(Color.accentColor.opacity(0.12))
                        .foregroundStyle(Color.accentColor)
                        .clipShape(Capsule())
                }
            }
        }
    }

    private func isMeaningfulTitle(_ title: String) -> Bool {
        !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Resolve screenshot IR paths (relative to the meeting dir or absolute)
    /// into validated file URLs under the meeting dir.
    private func resolvedScreenshots(_ screenshots: [Screenshot]) -> [URL] {
        guard !meetingDir.isEmpty else { return [] }
        return screenshots.compactMap { shot -> URL? in
            guard !shot.path.isEmpty else { return nil }
            let candidate = shot.path.hasPrefix("/")
                ? shot.path
                : (meetingDir as NSString).appendingPathComponent(shot.path)
            guard PathValidator.isPath(candidate, under: meetingDir),
                  FileManager.default.fileExists(atPath: candidate) else { return nil }
            return URL(fileURLWithPath: candidate)
        }
    }

    private var placeholder: some View {
        VStack {
            Spacer()
            Text("No notes").foregroundStyle(.secondary)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// A single action item rendered as a checkbox row. The checkbox is visual only
/// (local state, not persisted) per the structured-notes IR having no done flag.
private struct ActionItemRow: View {
    let item: ActionItem
    @State private var done = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Button {
                done.toggle()
            } label: {
                Image(systemName: done ? "checkmark.square.fill" : "square")
                    .foregroundStyle(done ? Color.accentColor : Color.secondary)
            }
            .buttonStyle(.plain)

            VStack(alignment: .leading, spacing: 2) {
                Text(Markdown.inlineAttributed(item.description))
                    .strikethrough(done, color: .secondary)
                    .textSelection(.enabled)
                if let detail = metaLine {
                    Text(detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var metaLine: String? {
        var parts: [String] = []
        if let owner = item.owner, !owner.isEmpty { parts.append(owner) }
        if let due = item.due, !due.isEmpty { parts.append("due \(due)") }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}
