import SwiftUI
import UniformTypeIdentifiers

/// The meeting workspace: a notes-primary, tabbed review experience.
///
/// Loads `notes.json` (structured IR, preferred), `notes.md` (markdown
/// fallback), `transcript.json`, and the `screenshots/` directory for the
/// selected meeting. A segmented control switches between Notes, Transcript,
/// and Info. The Notes tab also hosts this meeting's processing / error /
/// no-notes states, driven by the shared `AppViewModel` notes flow.
struct MeetingDetailView<Controller: RecordingController & ObservableObject>: View {
    let meeting: Meeting
    @ObservedObject var vm: AppViewModel
    let recordingController: Controller?
    let onRecordingStarted: (String) async throws -> Void
    let onRecordingStopped: (RecordingStopOutcome) async throws -> Void

    @State private var transcript: Transcript?
    @State private var notesContent: String?
    @State private var structuredNotes: StructuredNotes?
    @State private var screenshots: [URL]?
    @State private var isLoading = true
    @State private var selectedTab: DetailTab = .notes
    @State private var editedTitle: String = ""
    /// Last title successfully persisted via `meeting.rename`. Drives the
    /// "is the local edit dirty?" check so we only autosave when the title
    /// actually changed. Reset when the meeting changes.
    @State private var lastSavedTitle: String = ""
    @FocusState private var titleFocused: Bool
    @State private var showErrorDetails = false
    @State private var exportError: String?

    /// Cached video file URL, if present
    @State private var videoFileURL: URL?

    /// Guards against a rapid double-tap spawning two concurrent delete tasks
    /// (two confirm alerts, racy state).
    @State private var isDeletingVideo = false

    /// Tag names the user dismissed this session (not persisted). Prevents
    /// dismissed AI suggestions from reappearing until the view remounts.
    @State private var dismissedSuggestions: Set<String> = []

    /// Buffer for the "+ add your own" tag text field.
    @State private var newTagName: String = ""

    enum DetailTab: String, CaseIterable, Identifiable {
        case notes = "Notes"
        case transcript = "Transcript"
        case info = "Info"
        var id: String { rawValue }
    }

    init(
        meeting: Meeting,
        vm: AppViewModel,
        recordingController: Controller? = nil,
        onRecordingStarted: @escaping (String) async throws -> Void = { _ in },
        onRecordingStopped: @escaping (RecordingStopOutcome) async throws -> Void = { _ in }
    ) {
        self.meeting = meeting
        self.vm = vm
        self.recordingController = recordingController
        self.onRecordingStarted = onRecordingStarted
        self.onRecordingStopped = onRecordingStopped
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            if let controller = recordingController, meeting.endedAt == nil {
                Divider()
                RecordBar(
                    controller: controller,
                    meetingId: meeting.id,
                    onRecordingStarted: onRecordingStarted,
                    onRecordingStopped: onRecordingStopped
                )
            }
            Divider()
            Picker("View", selection: $selectedTab) {
                ForEach(DetailTab.allCases) { tab in
                    Text(tab.rawValue).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            Divider()

            switch selectedTab {
            case .notes: notesTab
            case .transcript: TranscriptView(transcript: transcript)
            case .info: infoTab
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: loadToken) { await loadMeetingData() }
        .onChange(of: meeting.id, initial: true) { _, _ in
            editedTitle = meeting.title
            lastSavedTitle = meeting.title
            dismissedSuggestions = []
            newTagName = ""
        }
        .alert("Export failed", isPresented: exportErrorPresented) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(exportError ?? "")
        }
    }

    private var exportErrorPresented: Binding<Bool> {
        Binding(
            get: { exportError != nil },
            set: { if !$0 { exportError = nil } }
        )
    }

    private var loadToken: String {
        "\(meeting.id)|\(meeting.endedAt ?? "")|\(meeting.durationMs ?? 0)|\(vm.notesReloadTick)"
    }

    // MARK: - Header

    private var header: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                // Inline-editable title. Autosaves on submit (Return) and on
                // focus-loss; the SaveStatusView across the row reflects the
                // save lifecycle (spinner, "Saved", or Retry on failure).
                TextField("Meeting title", text: $editedTitle)
                    .textFieldStyle(.plain)
                    .font(.title2.weight(.semibold))
                    .focused($titleFocused)
                    .disabled(vm.saveStatus.isSaving)
                    .onSubmit(commitTitleIfChanged)
                Spacer(minLength: 12)
                SaveStatusView(
                    status: vm.saveStatus,
                    retry: { handleSaveRetry() }
                )
                headerActions
            }

            Text(metadataLine)
                .font(.subheadline)
                .foregroundStyle(.secondary)

            FlowLayout(spacing: 6, lineSpacing: 6) {
                ForEach(trustChips) { chip in
                    TrustChip(systemImage: chip.icon, label: chip.label, tint: chip.tint)
                }
            }

            // Assigned tag chips (resolved from IDs) + pending AI suggestions.
            // Shows when notes exist (LLM proposed tags) OR any tags are
            // already assigned. The add-your-own field is always available.
            if structuredNotes != nil || !assignedTagNames.isEmpty {
                FlowLayout(spacing: 6, lineSpacing: 6) {
                    ForEach(assignedTagNames, id: \.self) { name in
                        Text(name)
                            .font(.caption)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 4)
                            .background(Color.accentColor.opacity(0.12))
                            .foregroundStyle(Color.accentColor)
                            .clipShape(Capsule())
                    }
                    ForEach(visiblePendingSuggestions, id: \.self) { name in
                        PendingTagChip(
                            name: name,
                            onAccept: { acceptSuggestion(name) },
                            onDismiss: { dismissedSuggestions.insert(name.lowercased()) }
                        )
                    }
                    AddTagField(
                        text: $newTagName,
                        onCommit: { commitNewTag() }
                    )
                }
            }
        }
        .padding(16)
        .onChange(of: titleFocused) { wasFocused, isFocused in
            // Save on blur (focus leaving the title field). `wasFocused` is
            // checked so we don't fire on the initial false → false state.
            if wasFocused && !isFocused {
                commitTitleIfChanged()
            }
        }
    }

    /// Route the shared SaveStatusView's Retry to the operation that actually
    /// failed. The chip is shared across title / notes / tag / org saves, so a
    /// hardcoded title retry was a no-op after a notes save failed. Notes route
    /// to a buffer-aware re-save; everything else falls back to the title path.
    private func handleSaveRetry() {
        switch vm.failedSaveOperation {
        case .notes:
            Task { await vm.retryNotesSave() }
        case .title, .other, .none:
            commitTitleIfChanged()
        }
    }

    /// Persist the edited title when it differs from `lastSavedTitle`. No-op
    /// when the title is unchanged or an autosave is already in flight. The
    /// view model drives the SaveStatus chip and patches `selectedMeeting`
    /// on success.
    private func commitTitleIfChanged() {
        let trimmed = editedTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed != lastSavedTitle else { return }
        guard !vm.saveStatus.isSaving else { return }
        let meetingId = meeting.id
        Task {
            if let updated = await vm.renameMeeting(id: meetingId, title: trimmed) {
                lastSavedTitle = updated.title
                editedTitle = updated.title
            }
            // On failure the view model keeps `saveStatus == .failed`; the
            // user's edited text stays in the field so Retry can re-submit.
        }
    }

    // MARK: - Tag chips (assigned + AI-suggested)

    /// Tag names currently assigned to the open meeting, resolved from IDs via
    /// the view model's cached tag list. Keyed by the actually-displayed
    /// meeting (`vm.selectedMeeting`) and resolved from the *unfiltered*
    /// `vm.allMeetings`: the sidebar-filtered `vm.meetings` can exclude the open
    /// meeting, which dropped the chips and made `pendingTags` re-suggest
    /// already-accepted tags. (Meeting itself doesn't carry tag IDs.)
    private var assignedTagNames: [String] {
        resolveAssignedTagNames(
            forMeeting: vm.selectedMeeting?.id ?? meeting.id,
            in: vm.allMeetings,
            tags: vm.tags
        )
    }

    /// LLM-proposed tag names minus already-assigned names minus session-
    /// dismissed names. Case-insensitive subtraction so accepting "Swift"
    /// hides the "swift" suggestion.
    private var visiblePendingSuggestions: [String] {
        guard let notes = structuredNotes else { return [] }
        let proposed = notes.frontmatter.tags
        let pending = pendingTags(proposed: proposed, assigned: assignedTagNames)
        let dismissed = dismissedSuggestions
        return pending.filter { !dismissed.contains($0.lowercased()) }
    }

    /// Accept an AI-suggested tag: creates it (idempotent) and assigns it.
    private func acceptSuggestion(_ name: String) {
        let meetingId = meeting.id
        Task {
            let ok = await vm.acceptTagSuggestion(
                meetingId: meetingId, name: name
            )
            if ok {
                dismissedSuggestions.insert(name.lowercased())
            }
        }
    }

    /// Commit the "+ add your own" text field: same create+assign path.
    private func commitNewTag() {
        let trimmed = newTagName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        let meetingId = meeting.id
        Task {
            let ok = await vm.acceptTagSuggestion(
                meetingId: meetingId, name: trimmed
            )
            if ok {
                newTagName = ""
            }
        }
    }

    private var headerActions: some View {
        HStack(spacing: 8) {
            Button {
                exportNotes()
            } label: {
                Label("Export", systemImage: "square.and.arrow.up")
            }
            .disabled(!hasNotes)
            .help("Export notes as Markdown")

            Button {
                vm.openInFinder(path: meeting.dirPath)
            } label: {
                Label("Reveal", systemImage: "folder")
            }
            .help("Reveal meeting folder in Finder")

            Button {
                // Sharing is a future, opt-in, local-first feature (post-v0.1); disabled for now.
            } label: {
                Label("Share", systemImage: "person.crop.circle.badge.plus")
            }
            .disabled(true)
            .help("Sharing isn't available yet")
        }
        .labelStyle(.iconOnly)
        .buttonStyle(.bordered)
    }

    // MARK: - Notes tab + states

    @ViewBuilder
    private var notesTab: some View {
        if isProcessingThisMeeting {
            processingView
        } else if let detail = errorDetailForThisMeeting {
            errorStateView(kind: detail.kind, message: detail.message)
        } else if isLoading {
            ProgressView("Loading…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if hasNotes {
            notesViewWithAutosave()
        } else {
            noNotesView
        }
    }

    /// Build the notes view with the autosave closure wired up. Isolated
    /// into its own method so the local lets that break the generic
    /// `MeetingDetailView<Controller>` capture don't have to live inside a
    /// `@ViewBuilder` block (which disallows local declarations).
    private func notesViewWithAutosave() -> some View {
        let viewModel = vm
        let meetingId = meeting.id
        return NotesView(
            notes: structuredNotes,
            markdownFallback: notesContent,
            meetingDir: meeting.dirPath,
            onSave: { [viewModel, meetingId] markdown in
                await viewModel.saveNotes(meetingId: meetingId, markdown: markdown)
            }
        )
        // Identity bound to the meeting + notes-reload tick: switching
        // meetings or regenerating notes remounts NotesView so its
        // internal edit state / overriddenMarkdown resets.
        .id("\(meeting.id)|\(vm.notesReloadTick)")
    }

    private var processingView: some View {
        let progress = vm.notesProgress
        return VStack(spacing: 16) {
            Spacer()
            if let progress, let current = progress.current, let total = progress.total, total > 0 {
                ProgressView(value: max(0.0, min(Double(current) / Double(total), 1.0)))
                    .frame(maxWidth: 320)
            } else {
                ProgressView()
            }
            Text(progress?.stageLabel ?? "Generating notes…").font(.headline)
            if let message = progress?.message, !message.isEmpty {
                Text(message).font(.subheadline).foregroundStyle(.secondary)
            }
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
    }

    private func errorStateView(kind: String, message: String) -> some View {
        VStack(spacing: 14) {
            Spacer()
            Image(systemName: "exclamationmark.triangle")
                .font(.system(size: 36))
                .foregroundStyle(.orange)
            Text("Notes generation failed").font(.headline)
            Text(friendlyError(kind: kind, message: message))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            HStack(spacing: 12) {
                Button("Retry") {
                    Task { await vm.generateNotes(for: meeting) }
                }
                .buttonStyle(.borderedProminent)
                Button(showErrorDetails ? "Hide details" : "Show details") {
                    showErrorDetails.toggle()
                }
            }
            if showErrorDetails {
                Text("\(kind): \(message)")
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .padding(8)
                    .frame(maxWidth: 420)
                    .background(Color.secondary.opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 6))
            }
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
    }

    private var noNotesView: some View {
        VStack(spacing: 14) {
            Spacer()
            Image(systemName: "doc.text")
                .font(.system(size: 36))
                .foregroundStyle(.secondary)
            Text("No notes yet").font(.headline)
            if let hint = vm.deferredNotesHint, hint.meetingId == meeting.id {
                // Auto-generate was requested at stop, but the engine couldn't
                // start the job (compute warming up). Gentle, non-error nudge;
                // the Generate Notes button below still works.
                Text(hint.message)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .multilineTextAlignment(.center)
            } else {
                Text("Generate structured notes from this meeting's audio.")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            Button {
                Task { await vm.generateNotes(for: meeting) }
            } label: {
                Label("Generate Notes", systemImage: "sparkles")
                    .padding(.horizontal, 6)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            // Only one notes job is tracked at a time; block a second start
            // (e.g. from another meeting) while one is in flight.
            .disabled(vm.isNotesJobActive)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
    }

    // MARK: - Info tab

    private var infoTab: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                infoRow("Model", transcript?.model)
                infoRow("Audio file", audioFileName)
                infoRow("Duration", durationMs.map { MeetingDate.duration(ms: $0) })
                infoRow("Speakers", speakerSummary)
                infoRow("Languages", languageSummary)
                infoRow("Tags", structuredNotes?.frontmatter.tags.isEmpty == false
                    ? structuredNotes?.frontmatter.tags.joined(separator: ", ")
                    : nil)
                infoRow("Template", nonEmpty(structuredNotes?.frontmatter.template))
                infoRow("Dialect", humanizedDialect)
                infoRow("Started", startedDate.map { "\(MeetingDate.shortDate($0)) · \(MeetingDate.shortTime($0))" })
                infoRow("Ended", endedDate.map { "\(MeetingDate.shortDate($0)) · \(MeetingDate.shortTime($0))" })
                infoRow("panops", nonEmpty(structuredNotes?.frontmatter.panopsVersion))
                if hasVideoFile, let videoFileURL {
                    videoRow(videoFileURL)
                }
            }
            .padding(16)
        }
    }

    @ViewBuilder
    private func infoRow(_ label: String, _ value: String?) -> some View {
        if let value, !value.isEmpty {
            HStack(alignment: .top) {
                Text(label)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(width: 110, alignment: .leading)
                Text(value)
                    .font(.subheadline)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.vertical, 6)
            Divider()
        }
    }

    @ViewBuilder
    private func videoRow(_ url: URL) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Recording")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(width: 110, alignment: .leading)
                Text(url.lastPathComponent)
                    .font(.subheadline)
                    .foregroundStyle(.primary)
                Spacer()
                Button {
                    playVideo()
                } label: {
                    Label("Play", systemImage: "play.circle")
                }
                .help("Play video")
                Button {
                    revealVideoInFinder()
                } label: {
                    Label("Reveal", systemImage: "folder")
                }
                .help("Reveal in Finder")
                Button {
                    Task {
                        await deleteVideo()
                    }
                } label: {
                    Label("Delete video", systemImage: "xmark.circle")
                        .foregroundStyle(.red)
                }
                .disabled(isDeletingVideo)
                .help("Delete video to reclaim space")
                .foregroundStyle(.red)
            }
            Divider()
            HStack {
                Text("Size")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(width: 110, alignment: .leading)
                Text(formatFileSize(for: url))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            .padding(.vertical, 6)
            Divider()
        }
    }

    // MARK: - Derived values

    private var hasNotes: Bool {
        if structuredNotes != nil { return true }
        if let md = notesContent, !md.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return true }
        return false
    }

    private var isProcessingThisMeeting: Bool {
        guard vm.notesGenMeetingId == meeting.id else { return false }
        if case .working = vm.state { return true }
        return false
    }

    private var errorDetailForThisMeeting: (kind: String, message: String)? {
        guard vm.notesGenMeetingId == meeting.id, case .error(let kind, let message) = vm.state else {
            return nil
        }
        return (kind, message)
    }

    private var durationMs: UInt64? {
        if let d = structuredNotes?.frontmatter.durationMs, d > 0 { return d }
        return meeting.durationMs
    }

    private var startedDate: Date? {
        MeetingDate.parse(structuredNotes?.frontmatter.startedAt ?? "")
            ?? MeetingDate.parse(meeting.startedAt)
    }

    private var endedDate: Date? {
        meeting.endedAt.flatMap { MeetingDate.parse($0) }
    }

    private var speakerCount: Int? {
        if let speakers = structuredNotes?.frontmatter.speakers, !speakers.isEmpty {
            return speakers.count
        }
        if let segments = transcript?.segments {
            let ids = Set(segments.compactMap { $0.speakerId })
            return ids.isEmpty ? nil : ids.count
        }
        return nil
    }

    private var speakerSummary: String? {
        if let speakers = structuredNotes?.frontmatter.speakers, !speakers.isEmpty {
            return speakers.joined(separator: ", ")
        }
        if let count = speakerCount { return "\(count)" }
        return nil
    }

    private var languageSummary: String? {
        if let langs = structuredNotes?.frontmatter.languages, !langs.isEmpty {
            return langs.map { $0.uppercased() }.joined(separator: ", ")
        }
        return nonEmpty(meeting.language).map { $0.uppercased() }
    }

    private var audioFileName: String? {
        if let path = transcript?.audioPath, !path.isEmpty {
            return (path as NSString).lastPathComponent
        }
        return structuredNotes?.frontmatter.sourceAudio.map { ($0 as NSString).lastPathComponent }
    }

    private var humanizedDialect: String? {
        guard let dialect = structuredNotes?.frontmatter.dialect, !dialect.isEmpty else { return nil }
        return dialect
            .split(separator: "-")
            .map { $0.prefix(1).uppercased() + $0.dropFirst() }
            .joined(separator: " ")
    }

    private var metadataLine: String {
        var parts: [String] = []
        if let date = startedDate {
            parts.append(MeetingDate.shortDate(date))
            parts.append(MeetingDate.shortTime(date))
        }
        if let ms = durationMs, ms > 0 { parts.append(MeetingDate.duration(ms: ms)) }
        if let lang = languageSummary { parts.append(lang) }
        if let count = speakerCount { parts.append(count == 1 ? "1 speaker" : "\(count) speakers") }
        return parts.isEmpty ? "Meeting" : parts.joined(separator: " · ")
    }

    private struct ChipData: Identifiable {
        let id = UUID()
        let icon: String
        let label: String
        let tint: Color
    }

    /// The Processing chip reflects the engine's resolved LLM provider so the
    /// trust strip never claims "Local" when the engine actually resolved to a
    /// cloud model. Mirrors the toolbar's LlmProviderChip (both read llmInfo).
    private var processingChip: ChipData {
        if let info = vm.llmInfo, !info.local {
            return ChipData(icon: "cloud", label: "Cloud", tint: .orange)
        }
        return ChipData(icon: "cpu", label: "Local", tint: .green)
    }

    private var trustChips: [ChipData] {
        var chips: [ChipData] = [
            processingChip,
            ChipData(icon: "lock", label: "Private", tint: .green),
        ]
        if transcript != nil || audioFileName != nil {
            chips.append(ChipData(icon: "waveform", label: "Audio", tint: .secondary))
        }
        if let count = screenshots?.count, count > 0 {
            chips.append(ChipData(icon: "photo", label: count == 1 ? "1 screenshot" : "\(count) screenshots", tint: .secondary))
        }
        if let lang = languageSummary {
            chips.append(ChipData(icon: "globe", label: lang, tint: .secondary))
        }
        return chips
    }

    private func nonEmpty(_ value: String?) -> String? {
        guard let value, !value.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        return value
    }

    private func friendlyError(kind: String, message: String) -> String {
        if kind == "no_audio" { return message }
        return "Something went wrong while generating notes. You can retry or check the details."
    }

    // MARK: - Video file handling

    private var hasVideoFile: Bool {
        videoFileURL != nil
    }

    private func formatFileSize(for url: URL) -> String {
        guard let attrs = try? url.resourceValues(forKeys: [.fileSizeKey]),
              let size = attrs.fileSize else {
            return "Unknown size"
        }
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        return formatter.string(fromByteCount: Int64(size))
    }

    private func playVideo() {
        guard let url = videoFileURL else { return }
        NSWorkspace.shared.open(url)
    }

    private func revealVideoInFinder() {
        guard let url = videoFileURL else { return }
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    @MainActor
    private func deleteVideo() async {
        // A rapid double-tap can enqueue two delete tasks before the button's
        // .disabled(isDeletingVideo) takes effect; bail on the second one.
        guard !isDeletingVideo else { return }
        guard let url = videoFileURL else { return }

        isDeletingVideo = true
        defer { isDeletingVideo = false }

        let alert = NSAlert()
        alert.messageText = "Delete video file?"
        alert.informativeText = "This will delete \(url.lastPathComponent) and reclaim disk space. This action cannot be undone."
        alert.addButton(withTitle: "Delete")
        alert.addButton(withTitle: "Cancel")

        guard alert.runModal() == .alertFirstButtonReturn else { return }

        // Route deletion through the engine's path-validated meeting.deleteVideo
        // IPC — the engine owns the meeting dir and validates the path. Do NOT
        // delete the file directly. Clear local state only after it succeeds.
        do {
            _ = try await vm.deleteVideoForMeeting(meetingId: meeting.id)
            videoFileURL = nil
        } catch {
            AppViewModel.logFullError("meeting.deleteVideo", error)
            let errorAlert = NSAlert()
            errorAlert.messageText = "Failed to delete video"
            errorAlert.informativeText = error.localizedDescription
            errorAlert.addButton(withTitle: "OK")
            errorAlert.runModal()
        }
    }

    // MARK: - Export

    private func exportNotes() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(editedTitle.isEmpty ? meeting.title : editedTitle).md"
        panel.allowedContentTypes = [UTType(filenameExtension: "md") ?? .plainText]
        guard panel.runModal() == .OK, let dest = panel.url else { return }

        // Prefer the already-loaded markdown; otherwise read notes.md from disk.
        var data = notesContent.map { Data($0.utf8) }
        if data == nil {
            let src = (meeting.dirPath as NSString).appendingPathComponent("notes.md")
            if PathValidator.isPath(src, under: meeting.dirPath) {
                data = FileManager.default.contents(atPath: src)
            }
        }
        guard let data else { return }
        do {
            try data.write(to: dest)
        } catch {
            AppViewModel.logFullError("notes.export", error)
            exportError = "Couldn't save notes to \(dest.lastPathComponent). \(error.localizedDescription)"
        }
    }

    // MARK: - Loading

    private func loadMeetingData() async {
        transcript = nil
        notesContent = nil
        structuredNotes = nil
        screenshots = nil
        videoFileURL = nil
        isLoading = true

        let dirPath = meeting.dirPath

        // Read + decode off the MainActor: transcript.json / notes.json can be
        // large, and a sync read+decode on the main thread hitches the UI on
        // every meeting switch.
        let loaded = await Task.detached(priority: .utility) {
            () -> (Transcript?, String?, StructuredNotes?, [URL]?, URL?) in
            var t: Transcript?
            var notesMd: String?
            var notesJson: StructuredNotes?
            var shots: [URL]?
            var videoURL: URL?

            let transcriptPath = (dirPath as NSString).appendingPathComponent("transcript.json")
            if PathValidator.isPath(transcriptPath, under: dirPath),
               FileManager.default.fileExists(atPath: transcriptPath),
               let data = FileManager.default.contents(atPath: transcriptPath) {
                t = try? JSONDecoder().decode(Transcript.self, from: data)
            }

            let notesJsonPath = (dirPath as NSString).appendingPathComponent("notes.json")
            if PathValidator.isPath(notesJsonPath, under: dirPath),
               FileManager.default.fileExists(atPath: notesJsonPath),
               let data = FileManager.default.contents(atPath: notesJsonPath) {
                notesJson = try? JSONDecoder().decode(StructuredNotes.self, from: data)
            }

            let notesMdPath = (dirPath as NSString).appendingPathComponent("notes.md")
            if PathValidator.isPath(notesMdPath, under: dirPath),
               FileManager.default.fileExists(atPath: notesMdPath) {
                notesMd = try? String(contentsOfFile: notesMdPath, encoding: .utf8)
            }

            let screenshotsPath = (dirPath as NSString).appendingPathComponent("screenshots")
            if PathValidator.isPath(screenshotsPath, under: dirPath),
               FileManager.default.fileExists(atPath: screenshotsPath),
               let contents = try? FileManager.default.contentsOfDirectory(atPath: screenshotsPath) {
                shots = contents
                    .filter { $0.hasSuffix(".png") || $0.hasSuffix(".jpg") || $0.hasSuffix(".jpeg") }
                    .compactMap { filename -> URL? in
                        let path = (screenshotsPath as NSString).appendingPathComponent(filename)
                        guard PathValidator.isPath(path, under: dirPath) else { return nil }
                        return URL(fileURLWithPath: path)
                    }
                    .sorted { $0.lastPathComponent < $1.lastPathComponent }
            }

            // Look for recording.mov in the meeting directory
            let videoPath = (dirPath as NSString).appendingPathComponent("recording.mov")
            if PathValidator.isPath(videoPath, under: dirPath),
               FileManager.default.fileExists(atPath: videoPath) {
                videoURL = URL(fileURLWithPath: videoPath)
            }

            return (t, notesMd, notesJson, shots, videoURL)
        }.value

        guard !Task.isCancelled else { return }

        transcript = loaded.0
        notesContent = loaded.1
        structuredNotes = loaded.2
        screenshots = loaded.3
        videoFileURL = loaded.4
        isLoading = false
    }
}

extension JobProgressEvent {
    /// Human-facing label for a notes-generation stage.
    var stageLabel: String {
        switch stage {
        case "loading":
            return "Loading audio…"
        case "transcribing":
            if let current, let total {
                return "Transcribing \(current)/\(total)…"
            }
            return "Transcribing…"
        case "diarizing":
            return "Identifying speakers…"
        case "generating_notes":
            return "Writing notes…"
        case "exporting":
            return "Saving…"
        default:
            return "Generating notes…"
        }
    }
}

// MARK: - AI tag suggestion chips

/// A pending AI-suggested tag chip: dashed outline (visually distinct from
/// the solid assigned-tag capsules), an accept button, and a dismiss button.
/// The dashed border signals "this isn't real yet" — accepting makes it solid.
struct PendingTagChip: View {
    let name: String
    let onAccept: () -> Void
    let onDismiss: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Text(name)
                .font(.caption)
            Button(action: onAccept) {
                Image(systemName: "plus")
                    .font(.caption2.weight(.bold))
            }
            .buttonStyle(.plain)
            .help("Accept tag")
            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .font(.caption2.weight(.bold))
            }
            .buttonStyle(.plain)
            .help("Dismiss suggestion")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .foregroundStyle(.orange)
        .overlay(
            Capsule()
                .strokeBorder(
                    style: StrokeStyle(lineWidth: 1, dash: [4, 3])
                )
                .foregroundStyle(.orange.opacity(0.6))
        )
        .clipShape(Capsule())
    }
}

/// Inline text field for adding a custom tag. Commits on Return, styled as
/// a small capsule with a "+" icon to match the pending-tag visual language.
struct AddTagField: View {
    @Binding var text: String
    let onCommit: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: "plus")
                .font(.caption2.weight(.bold))
                .foregroundStyle(.secondary)
            TextField("add tag", text: $text)
                .textFieldStyle(.plain)
                .font(.caption)
                .frame(width: 80)
                .onSubmit(onCommit)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .overlay(
            Capsule()
                .strokeBorder(
                    style: StrokeStyle(lineWidth: 1, dash: [4, 3])
                )
                .foregroundStyle(.secondary.opacity(0.5))
        )
        .clipShape(Capsule())
    }
}
