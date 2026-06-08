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
struct NotesView: View {
    let notes: StructuredNotes?
    let markdownFallback: String?
    let meetingDir: String

    init(notes: StructuredNotes?, markdownFallback: String?, meetingDir: String = "") {
        self.notes = notes
        self.markdownFallback = markdownFallback
        self.meetingDir = meetingDir
    }

    /// Convenience for markdown-only callers (legacy `NotesView(content:)`).
    init(content: String?) {
        self.init(notes: nil, markdownFallback: content, meetingDir: "")
    }

    var body: some View {
        if let notes {
            structuredDocument(notes)
        } else if let md = markdownFallback, !md.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            ScrollView {
                MarkdownBlocksView(markdown: Markdown.stripFrontmatter(md))
                    .padding(20)
            }
        } else {
            placeholder
        }
    }

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
