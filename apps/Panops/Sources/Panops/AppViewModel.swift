import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class AppViewModel: ObservableObject {
    enum State {
        case engineNotConnected
        case idle(audio: URL?)
        case working(meetingId: String, audioName: String)
        case done(notesPath: String)
        case error(kind: String, message: String)
    }

    /// Non-error nudge shown when a stopped recording requested auto-generate
    /// notes but the engine couldn't enqueue the job (compute wasn't ready).
    /// Meeting-scoped so it only renders for the meeting it belongs to.
    struct DeferredNotesHint: Equatable {
        let meetingId: String
        let message: String
    }

    /// Copy for the deferred-notes hint. Single source so the message stays
    /// consistent wherever the hint is shown.
    static let deferredNotesMessage =
        "Notes deferred — compute wasn't ready. Generate when ready."

    @Published var state: State = .idle(audio: nil)
    @Published var selectedMeetingId: String?
    @Published var activeRecordingMeetingId: String?
    /// Directory of the meeting that owns the active recording. The health-line
    /// byte poll keys on this — not the sidebar selection — so switching meetings
    /// mid-recording doesn't re-target the poll at an idle directory.
    @Published var activeRecordingDirPath: String?
    /// The displayed meeting list — `allMeetings` narrowed by the current
    /// sidebar selection. The content list renders this.
    @Published var meetings: [MeetingSummary] = []
    /// Monotonic token, bumped on every sidebar-selection change, used to drop
    /// out-of-order filtered-fetch results: an in-flight `meeting.list` that
    /// resolves after a newer selection started must not overwrite `meetings`.
    /// Main-actor guarded (the view model is `@MainActor`), so not `@Published`.
    private var loadGeneration: UInt64 = 0
    /// The full unfiltered meeting list (Phase B). Smart Views filter this
    /// client-side; space/project/tag selections refetch via the engine.
    @Published private(set) var allMeetings: [MeetingSummary] = []
    @Published var selectedMeeting: Meeting?
    /// Organization state (Phase B), loaded on connect and after edits.
    @Published var spaces: [Space] = []
    @Published var projects: [Project] = []
    @Published var tags: [Tag] = []
    /// Current sidebar selection; drives the displayed meeting list. Defaults
    /// to All (every meeting) so first launch matches the pre-Phase-B view.
    @Published var sidebarSelection: SidebarSelection = .smart(.all)
    @Published var notesProgress: JobProgressEvent?
    @Published var llmInfo: LlmInfo?
    /// Autosave lifecycle for the current edit (title, notes). Driven by
    /// `renameMeeting` and `saveNotes`; consumed by `SaveStatusView` in the
    /// meeting workspace header / notes toolbar.
    @Published var saveStatus: SaveStatus = .idle
    /// Which edit produced the current `.failed` saveStatus. The single
    /// `SaveStatusView` is shared across title / notes / tag / org saves, so its
    /// Retry must route to the operation that actually failed — a hardcoded
    /// title retry re-ran the title save even after a *notes* save failed (a
    /// no-op). Set in each save's failure path; only read while `isFailed`.
    @Published private(set) var failedSaveOperation: SaveOperation?
    /// Inputs of the last `saveNotes` call, kept so a header-driven Retry can
    /// re-run the notes save (the editor buffer lives inside NotesView and
    /// isn't reachable from the shared status chip).
    @Published private(set) var lastNotesAttempt: NotesAttempt?
    /// Which meeting the current/last notes generation targets. Lets the meeting
    /// workspace show processing/error inline for the right meeting (the audio-
    /// file flow targets a freshly-created meeting that isn't selected, so its
    /// states render in the no-selection area instead).
    @Published var notesGenMeetingId: String?
    /// Bumped when notes generation completes so the open meeting workspace
    /// re-reads `notes.json` from disk.
    @Published private(set) var notesReloadTick: Int = 0
    /// Set when a stopped recording requested auto-generate but the engine
    /// deferred it (compute warming up). The meeting workspace shows it as a
    /// gentle hint; the meeting stays manually generable.
    @Published var deferredNotesHint: DeferredNotesHint?

    private let client: IpcClient
    private var pollingTask: Task<Void, Never>?
    private let eventStream: EventStreamActor
    private var wsSubscriptionTask: Task<Void, Never>?
    private var notesLastProgressAt: Date?
    nonisolated private static let progressSilenceTimeoutSeconds: TimeInterval = 5 * 60
    nonisolated private static let wsSetupTimeoutNanoseconds: UInt64 = 3_000_000_000

    private enum WsSetupResult: Sendable {
        case succeeded
        case failed
        case timedOut
    }

    init(client: IpcClient) {
        self.client = client
        self.eventStream = EventStreamActor()
    }

    func connect() async throws {
        try await client.connect()
        await loadServerInfoBestEffort()
        await refreshOrganization()
        await refreshMeetingsWithStartupRetry()
    }

    /// Retry connection to engine after a previous failure.
    /// Used when state is `.engineNotConnected`.
    func retryConnect() async {
        do {
            try await client.connect()
            await loadServerInfoBestEffort()
            await refreshOrganization()
            await refreshMeetingsWithStartupRetry()
            state = .idle(audio: nil)
        } catch {
            Self.logFullError("ipc.connect.retry", error)
            // Stay in engineNotConnected state
        }
    }

    func setEngineNotConnected() {
        state = .engineNotConnected
    }

    func pickAudio() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowedContentTypes = [
            UTType.wav,
            UTType(filenameExtension: "m4a") ?? UTType.audio,
            UTType.mp3,
            UTType.movie,
        ]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        state = .idle(audio: url)
    }

    /// Audio-file flow: create a fresh meeting from a picked file and generate.
    func generate() async {
        guard case .idle(let audio?) = state else { return }
        do {
            let meetingId = try await client.meetingStart()
            await beginNotesGeneration(meetingId: meetingId, audio: audio)
        } catch let IpcClientError.rpcError(_, message) {
            failNotesStart(kind: "rpc_error", message: message)
        } catch {
            Self.logFullError("meeting.start", error)
            failNotesStart(kind: "internal", message: "Could not reach the engine.")
        }
    }

    /// Selected-meeting flow: generate (or retry) notes for an existing meeting
    /// using audio already captured in its directory. Targets the meeting's own
    /// id rather than creating a new one.
    func generateNotes(for meeting: Meeting) async {
        // Only one notes job is tracked at a time (notesGenMeetingId + state).
        // Starting a second while one runs would clobber the first's progress
        // and completion tracking, so refuse while any job is in flight.
        guard !isNotesJobActive else { return }
        guard let audio = locateAudio(in: meeting.dirPath) else {
            notesGenMeetingId = meeting.id
            notesProgress = nil
            notesLastProgressAt = nil
            state = .error(
                kind: "no_audio",
                message: "No recorded audio was found for this meeting."
            )
            return
        }
        await beginNotesGeneration(meetingId: meeting.id, audio: audio)
    }

    /// Shared notes-generation machinery used by both entry points: subscribe
    /// (or fall back to polling), kick off `notes.generate`, register the
    /// completion callback, and arm the polling safety net.
    private func beginNotesGeneration(meetingId: String, audio: URL) async {
        do {
            notesProgress = nil
            notesLastProgressAt = Date()
            notesGenMeetingId = meetingId
            deferredNotesHint = nil

            // Ensure WebSocket subscription is active (lazy).
            // If WebSocket fails, fall back to filesystem polling (fix #5).
            let wsOk = await ensureWsSubscription()

            let jobId = try await client.notesGenerate(audio: audio, meetingId: meetingId)
            state = .working(meetingId: meetingId, audioName: audio.lastPathComponent)
            await watchNotesJob(meetingId: meetingId, jobId: jobId, wsOk: wsOk)
        } catch let IpcClientError.rpcError(_, message) {
            failNotesStart(kind: "rpc_error", message: message)
        } catch {
            Self.logFullError("notes.generate", error)
            failNotesStart(kind: "internal", message: "Could not reach the engine.")
        }
    }

    /// Wire a notes job (manual or auto-generated) into the same completion
    /// tracking: a WebSocket callback for `job.done` / `job.error` / progress,
    /// plus filesystem polling as both the no-WebSocket fallback and the
    /// safety net if the stream ends before a terminal event. Callers set
    /// `state = .working` first; this only attaches the watchers.
    private func watchNotesJob(meetingId: String, jobId: String, wsOk: Bool) async {
        if wsOk {
            // Register callback for job completion (event-driven). Keep the
            // polling guard active too: if the WebSocket stream ends before
            // a terminal event arrives, the UI must not stay working forever.
            await eventStream.registerCallback(jobId: jobId, handler: { [weak self] event in
                Task { @MainActor in
                    switch event {
                    case .jobDone(_, let result):
                        self?.finishGenerationDone(
                            meetingId: result.meetingId,
                            jobId: jobId,
                            notesPath: result.primaryFile
                        )
                    case .jobError(_, let payload):
                        self?.recordNotesProgressHeartbeat()
                        self?.finishGenerationError(
                            meetingId: meetingId,
                            jobId: jobId,
                            kind: payload.kind,
                            message: payload.message
                        )
                    case .jobProgress(let progress):
                        self?.updateNotesProgress(progress)
                    case .unknown:
                        break
                    }
                }
            })
        }
        // Filesystem polling is also the WebSocket safety net. It is
        // cancelled by the terminal WebSocket callback when one arrives.
        startPolling(meetingId: meetingId, jobId: wsOk ? jobId : nil)
    }

    /// After a live recording stops, route any engine-enqueued auto-notes job
    /// into the same tracked flow as the manual button — or, when auto was
    /// requested but the engine deferred it (no job id), surface a gentle hint
    /// and leave the meeting manually generable.
    func handleAutoNotesAfterStop(meetingId: String, outcome: RecordingStopOutcome) async {
        deferredNotesHint = nil
        if let jobId = outcome.notesJobId {
            await trackAutoGeneratedNotes(
                meetingId: meetingId,
                jobId: jobId,
                audioName: outcome.audioURL?.lastPathComponent ?? "Recording"
            )
        } else if outcome.autoGenerateNotesRequested {
            deferredNotesHint = DeferredNotesHint(
                meetingId: meetingId,
                message: Self.deferredNotesMessage
            )
        }
    }

    /// Attach the tracked notes-generation flow to a job the engine already
    /// enqueued at `recording.stop`. Reuses `watchNotesJob` (no duplicated
    /// job-watching logic); the only difference from the manual path is that
    /// the job id is known up front instead of returned by `notes.generate`.
    private func trackAutoGeneratedNotes(meetingId: String, jobId: String, audioName: String) async {
        // Only one notes job is tracked at a time. If one is already in flight,
        // skip — the engine job still runs and the sidebar reflects has_notes
        // once it lands.
        guard !isNotesJobActive else { return }
        notesProgress = nil
        notesLastProgressAt = Date()
        notesGenMeetingId = meetingId
        deferredNotesHint = nil
        // Set .working before awaiting the (lazy) WebSocket setup so the UI
        // shows processing immediately for an auto-generated job.
        state = .working(meetingId: meetingId, audioName: audioName)
        let wsOk = await ensureWsSubscription()
        await watchNotesJob(meetingId: meetingId, jobId: jobId, wsOk: wsOk)
    }

    private func failNotesStart(kind: String, message: String) {
        notesProgress = nil
        notesLastProgressAt = nil
        // Keep notesGenMeetingId so the workspace can show the error inline.
        state = .error(kind: kind, message: message)
    }

    /// Find captured audio in a meeting directory. Live capture writes
    /// `system.wav` / `mic.wav`; prefer those, then any audio-like file.
    private func locateAudio(in dirPath: String) -> URL? {
        let fm = FileManager.default
        let preferred = ["system.wav", "mic.wav", "audio.wav"]
        for name in preferred {
            let path = (dirPath as NSString).appendingPathComponent(name)
            if PathValidator.isPath(path, under: dirPath), fm.fileExists(atPath: path) {
                return URL(fileURLWithPath: path)
            }
        }
        let audioExtensions: Set<String> = ["wav", "m4a", "mp3", "mov"]
        if let contents = try? fm.contentsOfDirectory(atPath: dirPath) {
            for name in contents.sorted()
            where audioExtensions.contains((name as NSString).pathExtension.lowercased()) {
                let path = (dirPath as NSString).appendingPathComponent(name)
                if PathValidator.isPath(path, under: dirPath) {
                    return URL(fileURLWithPath: path)
                }
            }
        }
        return nil
    }

    /// True while any notes-generation job is in flight. Only one job is tracked
    /// at a time, so callers must not start a second while this is true.
    var isNotesJobActive: Bool {
        if case .working = state { return true }
        return false
    }

    /// Lifecycle status for a meeting summary, used by the sidebar status pill.
    func status(for summary: MeetingSummary) -> MeetingStatus {
        if activeRecordingMeetingId == summary.id { return .recording }
        if notesGenMeetingId == summary.id, case .working = state { return .processing }
        if summary.hasNotes { return .ready }
        if summary.endedAt != nil { return .needsNotes }
        // Older payloads (pre ended_at) decode endedAt as nil; a positive
        // recorded duration still means the meeting ended and needs notes.
        if summary.durationMs > 0 { return .needsNotes }
        return .draft
    }

    /// Delete a meeting via the existing `meeting.delete` path; clear selection
    /// if it was the open one, then refresh the list.
    func deleteMeeting(id: String) async {
        do {
            try await client.meetingDelete(id: id)
            // Only clear the selection / empty the workspace once the delete
            // actually succeeded; a failed delete must leave the meeting open.
            if selectedMeetingId == id {
                selectedMeetingId = nil
                selectedMeeting = nil
                state = .idle(audio: nil)
            }
        } catch {
            Self.logFullError("meeting.delete", error)
        }
        await refreshMeetings()
    }

    /// Delete a meeting's video file via `meeting.deleteVideo`; does not remove
    /// the meeting row itself. Returns the deleted status and freed bytes.
    func deleteVideoForMeeting(meetingId: String) async throws -> (deleted: Bool, freedBytes: UInt64) {
        try await client.meetingDeleteVideo(meetingId: meetingId)
    }

    /// Fetch the engine's list of capturable windows for the New Recording
    /// sheet's window picker. Thin passthrough to the IPC client (mirrors
    /// `deleteVideoForMeeting`), so the sheet can be handed a real fetch closure
    /// without exposing the private client.
    func captureWindows() async throws -> [WindowInfo] {
        try await client.captureWindows()
    }

    /// Open a path (typically a meeting directory) in Finder, guarded to the
    /// panops data dir.
    func openInFinder(path: String) {
        guard PathValidator.isUnderPanopsDataDir(path) else {
            Self.logFullError(
                "openInFinder",
                NSError(domain: "PanopsShell", code: 1, userInfo: [NSLocalizedDescriptionKey: "refusing to open path outside panops data dir: \(path)"])
            )
            return
        }
        NSWorkspace.shared.open(URL(fileURLWithPath: path).standardizedFileURL)
    }

    /// Ensure WebSocket subscription is active. Lazy per spec decision.
    /// Returns true if WebSocket connected successfully, false on failure or
    /// timeout so the caller can start filesystem polling instead of hanging.
    private func ensureWsSubscription() async -> Bool {
        // Only subscribe once
        guard wsSubscriptionTask == nil else { return true }

        switch await Self.runWsSetupWithTimeout(client: client, eventStream: eventStream) {
        case .succeeded:
            wsSubscriptionTask = Task {
                // EventStreamActor.subscribe handles the stream internally
            }
            return true
        case .failed:
            // WebSocket failure is non-fatal; caller falls back to polling
            return false
        case .timedOut:
            Self.logFullError(
                "ws.subscribe",
                IpcClientError.websocketUpgradeFailed("WebSocket setup timed out")
            )
            // WebSocket stall is non-fatal; caller falls back to polling
            return false
        }
    }

    /// Race WebSocket setup against a short timer. This intentionally uses
    /// unstructured tasks rather than a task group because a stalled Network
    /// continuation may ignore cancellation; the timeout must still let the UI
    /// fall through to polling.
    nonisolated private static func runWsSetupWithTimeout(
        client: IpcClient,
        eventStream: EventStreamActor
    ) async -> WsSetupResult {
        let stream = AsyncStream<WsSetupResult> { continuation in
            let setupTask = Task {
                do {
                    try await client.wsConnect()
                    try Task.checkCancellation()
                    try await eventStream.subscribe(client: client)
                    continuation.yield(.succeeded)
                } catch {
                    Self.logFullError("ws.subscribe", error)
                    continuation.yield(.failed)
                }
                continuation.finish()
            }

            let timeoutTask = Task {
                try? await Task.sleep(nanoseconds: Self.wsSetupTimeoutNanoseconds)
                guard !Task.isCancelled else { return }
                setupTask.cancel()
                await eventStream.stop()
                await client.disconnect()
                continuation.yield(.timedOut)
                continuation.finish()
            }

            continuation.onTermination = { @Sendable _ in
                setupTask.cancel()
                timeoutTask.cancel()
            }
        }

        for await result in stream {
            return result
        }
        return .timedOut
    }

    /// Fetch engine status once after IPC connection. Best-effort: older or
    /// unhealthy engines simply omit the chip instead of blocking the app.
    private func loadServerInfoBestEffort() async {
        do {
            let info = try await client.serverInfo()
            llmInfo = info.llm
        } catch {
            Self.logFullError("ipc.server.info", error)
            llmInfo = nil
        }
    }

    /// Fetch meeting list from engine. Called on app launch and refresh.
    func refreshMeetings() async {
        await refreshMeetings(maxAttempts: 1, initialDelayMs: 0)
    }

    /// Fetch meeting list during startup after IPC connects. The launch-time
    /// ContentView task can race engine bootstrap; this retry ensures a
    /// transient not-ready engine does not leave the sidebar empty forever.
    private func refreshMeetingsWithStartupRetry() async {
        await refreshMeetings(maxAttempts: 4, initialDelayMs: 200)
    }

    private func refreshMeetings(maxAttempts: Int, initialDelayMs: UInt64) async {
        var delayMs = initialDelayMs
        for attempt in 1...maxAttempts {
            do {
                allMeetings = try await client.meetingList()
                await applySidebarSelection()
                return
            } catch {
                Self.logFullError("meeting.list", error)
                guard attempt < maxAttempts else { return }
                if delayMs > 0 {
                    try? await Task.sleep(nanoseconds: delayMs * 1_000_000)
                    delayMs = min(delayMs * 2, 1_000)
                }
            }
        }
    }

    // MARK: - Organization (Phase B): load + filtering + CRUD + assign

    /// Load spaces / projects / tags. Best-effort like `loadServerInfoBestEffort`:
    /// an older or unhealthy engine simply leaves the sections empty rather than
    /// blocking the app.
    func refreshOrganization() async {
        do {
            spaces = Self.sortedSpaces(try await client.spaceList())
        } catch {
            Self.logFullError("space.list", error)
        }
        do {
            projects = try await client.projectList()
        } catch {
            Self.logFullError("project.list", error)
        }
        do {
            tags = (try await client.tagList()).sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        } catch {
            Self.logFullError("tag.list", error)
        }
    }

    /// Recompute the displayed `meetings` for the current sidebar selection.
    /// Smart Views filter the loaded `allMeetings` client-side; space / project
    /// / tag selections refetch through the engine's `meeting.list` filters.
    func applySidebarSelection() async {
        // Bump the generation for every selection (incl. the synchronous Smart
        // View path) so any older in-flight filtered fetch is dropped when it
        // resolves — prevents a stale fetch clobbering the current list.
        loadGeneration &+= 1
        let token = loadGeneration
        switch sidebarSelection {
        case .smart(let view):
            meetings = meetingsForSmartView(view)
        case .space(let id):
            await loadFilteredMeetings(MeetingListParams(spaceId: id), token: token)
        case .project(let id):
            await loadFilteredMeetings(MeetingListParams(projectId: id), token: token)
        case .tag(let id):
            await loadFilteredMeetings(MeetingListParams(tagId: id), token: token)
        }
    }

    private func loadFilteredMeetings(_ filter: MeetingListParams, token: UInt64) async {
        do {
            let result = try await client.meetingList(filter: filter)
            // Drop the result if a newer selection superseded us mid-flight.
            guard token == loadGeneration else { return }
            meetings = result
        } catch {
            guard token == loadGeneration else { return }
            Self.logFullError("meeting.list.filter", error)
            meetings = []
        }
    }

    private func meetingsForSmartView(_ view: SmartView) -> [MeetingSummary] {
        switch view {
        case .all:
            return allMeetings
        case .inbox:
            return allMeetings.filter { $0.spaceId == nil }
        case .needsNotes:
            return allMeetings.filter { status(for: $0) == .needsNotes }
        case .thisWeek:
            return allMeetings.filter { isInCurrentWeek($0) }
        }
    }

    private func isInCurrentWeek(_ summary: MeetingSummary) -> Bool {
        guard let date = MeetingDate.parse(summary.startedAt) else { return false }
        let calendar = Calendar.current
        guard let week = calendar.dateInterval(of: .weekOfYear, for: Date()) else { return false }
        return week.contains(date)
    }

    /// Projects within a space, ordered for display.
    func projects(in spaceId: String) -> [Project] {
        Self.sortedProjects(projects.filter { $0.spaceId == spaceId })
    }

    /// Resolve a tag id to its display name (falls back to the id).
    func tagName(_ id: String) -> String {
        tags.first { $0.id == id }?.name ?? id
    }

    private static func sortedSpaces(_ items: [Space]) -> [Space] {
        items.sorted { ($0.position, $0.name) < ($1.position, $1.name) }
    }

    private static func sortedProjects(_ items: [Project]) -> [Project] {
        items.sorted { ($0.position, $0.name) < ($1.position, $1.name) }
    }

    func createSpace(name: String) async {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            let space = try await client.spaceCreate(name: trimmed)
            await refreshOrganization()
            sidebarSelection = .space(space.id)
            await applySidebarSelection()
        } catch {
            Self.logFullError("space.create", error)
        }
    }

    func renameSpace(id: String, name: String) async {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            try await client.spaceRename(id: id, name: trimmed)
            await refreshOrganization()
        } catch {
            Self.logFullError("space.rename", error)
        }
    }

    func deleteSpace(id: String) async {
        do {
            try await client.spaceDelete(id: id)
        } catch {
            Self.logFullError("space.delete", error)
            return
        }
        if isSelectionUnder(spaceId: id) {
            sidebarSelection = .smart(.all)
        }
        await refreshOrganization()
        await refreshMeetings()
    }

    func createProject(spaceId: String, name: String) async {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            let project = try await client.projectCreate(spaceId: spaceId, name: trimmed)
            await refreshOrganization()
            sidebarSelection = .project(project.id)
            await applySidebarSelection()
        } catch {
            Self.logFullError("project.create", error)
        }
    }

    func renameProject(id: String, name: String) async {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            try await client.projectRename(id: id, name: trimmed)
            await refreshOrganization()
        } catch {
            Self.logFullError("project.rename", error)
        }
    }

    func deleteProject(id: String) async {
        do {
            try await client.projectDelete(id: id)
        } catch {
            Self.logFullError("project.delete", error)
            return
        }
        if case .project(let pid) = sidebarSelection, pid == id {
            sidebarSelection = .smart(.all)
        }
        await refreshOrganization()
        await refreshMeetings()
    }

    func createTag(name: String) async {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            _ = try await client.tagCreate(name: trimmed)
            await refreshOrganization()
        } catch {
            Self.logFullError("tag.create", error)
        }
    }

    func deleteTag(id: String) async {
        do {
            try await client.tagDelete(id: id)
        } catch {
            Self.logFullError("tag.delete", error)
            return
        }
        if case .tag(let tid) = sidebarSelection, tid == id {
            sidebarSelection = .smart(.all)
        }
        await refreshOrganization()
        await refreshMeetings()
    }

    /// Assign a meeting to a space/project (both `nil` ⇒ move to Inbox), then
    /// refresh so the list and tag/space chips reflect the change. Drives
    /// `saveStatus` so the status chip surfaces success/failure.
    func assignMeeting(meetingId: String, spaceId: String?, projectId: String?) async {
        saveStatus = .saving
        do {
            try await client.meetingAssign(meetingId: meetingId, spaceId: spaceId, projectId: projectId)
            saveStatus = .saved
        } catch {
            Self.logFullError("meeting.assign", error)
            saveStatus = .failed(message: Self.describeSaveError(error, operation: "move meeting"))
            failedSaveOperation = .other
            return
        }
        await refreshMeetings()
    }

    /// Attach a tag to a meeting. Drives `saveStatus` so the status chip
    /// reflects success/failure, then refreshes the list so tag chips update.
    func addTag(meetingId: String, tagId: String) async {
        saveStatus = .saving
        do {
            try await client.tagAssign(meetingId: meetingId, tagId: tagId)
            saveStatus = .saved
        } catch {
            Self.logFullError("tag.assign", error)
            saveStatus = .failed(message: Self.describeSaveError(error, operation: "add tag"))
            failedSaveOperation = .other
            return
        }
        await refreshMeetings()
    }

    func removeTag(meetingId: String, tagId: String) async {
        do {
            try await client.tagUnassign(meetingId: meetingId, tagId: tagId)
        } catch {
            Self.logFullError("tag.unassign", error)
            return
        }
        await refreshMeetings()
    }

    /// Accept an AI-proposed tag suggestion. Creates the tag (idempotent on
    /// name) via `ipc.tag.create`, then attaches it to the meeting via
    /// `ipc.tag.assign`. Drives `saveStatus` so the header chip reflects the
    /// lifecycle. Refreshes org + meetings so the tag moves from pending to
    /// assigned. Returns true on success.
    @discardableResult
    func acceptTagSuggestion(meetingId: String, name: String) async -> Bool {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        saveStatus = .saving
        do {
            let tag = try await client.tagCreate(name: trimmed)
            try await client.tagAssign(meetingId: meetingId, tagId: tag.id)
            saveStatus = .saved
        } catch {
            Self.logFullError("tag.acceptSuggestion", error)
            saveStatus = .failed(
                message: Self.describeSaveError(error, operation: "accept tag")
            )
            failedSaveOperation = .other
            return false
        }
        await refreshOrganization()
        await refreshMeetings()
        return true
    }

    /// Execute a meeting-row drop on an org sidebar row. Maps the drop target
    /// to the right IPC call (meeting.assign for spaces/projects, tag.assign
    /// for tags), drives saveStatus, then refreshes org + meetings.
    func performDrop(meetingId: String, target: MeetingDropTarget) async {
        saveStatus = .saving
        do {
            switch target {
            case .space(let spaceId):
                try await client.meetingAssign(
                    meetingId: meetingId, spaceId: spaceId, projectId: nil
                )
            case .project(let projectId):
                // Engine sets the space from the project.
                try await client.meetingAssign(
                    meetingId: meetingId, spaceId: nil, projectId: projectId
                )
            case .tag(let tagId):
                try await client.tagAssign(meetingId: meetingId, tagId: tagId)
            }
            saveStatus = .saved
        } catch {
            Self.logFullError("drop.assign", error)
            saveStatus = .failed(
                message: Self.describeSaveError(
                    error, operation: "move to \(target.operationNoun)"
                )
            )
            failedSaveOperation = .other
            return
        }
        await refreshOrganization()
        await refreshMeetings()
    }

    // MARK: - Editing & autosave (editing-save slice, stage 2)

    /// Identifies which edit produced a `.failed` saveStatus so the shared
    /// `SaveStatusView` routes Retry to the right operation.
    enum SaveOperation: Equatable {
        case title
        case notes
        /// Tag / org assignments — they share the status chip, but their action
        /// originates outside the header so Retry there has nothing to re-run.
        case other
    }

    /// Captured inputs of the last notes save, for a header-driven Retry.
    struct NotesAttempt: Equatable {
        let meetingId: String
        let markdown: String
    }

    /// Re-run the most recent notes save — the Retry target when a `notes.save`
    /// failed. On success, bumps `notesReloadTick` so the open NotesView
    /// remounts and renders the just-saved markdown from disk.
    func retryNotesSave() async {
        guard let attempt = lastNotesAttempt else { return }
        if await saveNotes(meetingId: attempt.meetingId, markdown: attempt.markdown) {
            notesReloadTick += 1
        }
    }

    /// Rename a meeting via `ipc.meeting.rename`. Drives `saveStatus` so the
    /// header chip reflects the save lifecycle. On success, patches
    /// `selectedMeeting` in place and refreshes the list so the sidebar title
    /// updates. On failure, leaves the caller's edited text intact and
    /// surfaces the error for Retry. Returns the updated meeting on success.
    @discardableResult
    func renameMeeting(id: String, title: String) async -> Meeting? {
        saveStatus = .saving
        do {
            let updated = try await client.renameMeeting(meetingId: id, title: title)
            if selectedMeeting?.id == id {
                selectedMeeting = updated
            }
            saveStatus = .saved
            await refreshMeetings()
            return updated
        } catch {
            Self.logFullError("meeting.rename", error)
            saveStatus = .failed(message: Self.describeSaveError(error, operation: "save title"))
            failedSaveOperation = .title
            return nil
        }
    }

    /// Persist a manual edit to notes markdown via `ipc.notes.save`. Drives
    /// `saveStatus`. Returns true on success so the caller can swap the
    /// rendered view to the just-saved markdown; on failure the caller keeps
    /// the edited text and surfaces Retry.
    func saveNotes(meetingId: String, markdown: String) async -> Bool {
        saveStatus = .saving
        lastNotesAttempt = NotesAttempt(meetingId: meetingId, markdown: markdown)
        do {
            try await client.saveNotes(meetingId: meetingId, markdown: markdown)
            saveStatus = .saved
            await refreshMeetings()
            return true
        } catch {
            Self.logFullError("notes.save", error)
            saveStatus = .failed(message: Self.describeSaveError(error, operation: "save notes"))
            failedSaveOperation = .notes
            return false
        }
    }

    /// Failure copy for save-status chips. Pulls the RPC message when present
    /// so the user sees the engine's reason; falls back to a short generic
    /// line for network / transport errors.
    private static func describeSaveError(_ error: Error, operation: String) -> String {
        if case let IpcClientError.rpcError(_, message) = error, !message.isEmpty {
            return "Couldn't \(operation): \(message)"
        }
        return "Couldn't \(operation)."
    }

    private func isSelectionUnder(spaceId: String) -> Bool {
        switch sidebarSelection {
        case .space(let id):
            return id == spaceId
        case .project(let pid):
            return projects.first { $0.id == pid }?.spaceId == spaceId
        case .smart, .tag:
            return false
        }
    }

    /// Create a meeting through the existing `ipc.meeting.start` path, start
    /// live capture for it, select it in the sidebar, and refresh the list.
    /// `setup` carries the New Recording sheet's choices: title + language flow
    /// into `meeting.start`, audio sources + screenshot sampling into
    /// `recording.start`.
    func startNewRecording<Controller: RecordingController>(
        using recordingController: Controller,
        setup: RecordingSetup = .default
    ) async throws {
        let meetingId = try await client.meetingStart(config: setup.meetingConfig)
        do {
            try await recordingController.start(meetingId: meetingId, options: setup.recordingOptions)
        } catch {
            let recordingStartError = error
            do {
                // No recording was accepted, so remove the provisional row and
                // meeting directory rather than leaving a bogus open meeting.
                try await client.meetingDelete(id: meetingId)
            } catch {
                Self.logFullError("meeting.delete.cleanup", error)
            }
            await refreshMeetings()
            throw recordingStartError
        }

        activeRecordingMeetingId = meetingId
        selectedMeetingId = meetingId
        selectedMeeting = nil
        state = .idle(audio: nil)
        await refreshMeetings()
        await loadSelectedMeeting()
        // Pin the recording's directory so the health-line poll keeps targeting
        // it even if the sidebar selection changes mid-recording.
        if selectedMeeting?.id == meetingId {
            activeRecordingDirPath = selectedMeeting?.dirPath
        }
    }

    /// Load meeting detail when selected.
    func loadSelectedMeeting() async {
        guard let id = selectedMeetingId else {
            selectedMeeting = nil
            return
        }
        // New meeting selected — clear any prior autosave status so the status
        // chip doesn't carry over from the previous meeting's last edit.
        saveStatus = .idle
        do {
            let meeting = try await client.meetingGet(id: id)
            guard selectedMeetingId == id else { return }
            selectedMeeting = meeting
            showSelectedMeetingAfterTerminalState()
        } catch {
            Self.logFullError("meeting.get", error)
            selectedMeeting = nil
        }
    }

    /// Close the meeting row after a live recording stops, then refresh list
    /// and detail so ended_at/duration_ms are visible immediately.
    func finishLiveRecording(meetingId: String) async throws {
        let stoppedMeeting = try await client.meetingStop(id: meetingId)
        if activeRecordingMeetingId == meetingId {
            activeRecordingMeetingId = nil
            activeRecordingDirPath = nil
        }
        await refreshMeetings()
        if selectedMeetingId == meetingId {
            selectedMeeting = stoppedMeeting
            state = .idle(audio: nil)
        }
    }

    /// Close whichever meeting owns the active recording, regardless of the
    /// sidebar selection when the user presses Stop, then route any
    /// auto-generated notes job into the tracked flow (or show a deferred hint).
    func finishActiveLiveRecording(outcome: RecordingStopOutcome) async throws {
        guard let meetingId = activeRecordingMeetingId else { return }
        try await finishLiveRecording(meetingId: meetingId)
        await handleAutoNotesAfterStop(meetingId: meetingId, outcome: outcome)
    }

    private func showSelectedMeetingAfterTerminalState() {
        switch state {
        case .done, .error:
            state = .idle(audio: nil)
        case .engineNotConnected, .idle, .working:
            break
        }
    }

    /// Log the full error to stderr.
    nonisolated static func logFullError(_ op: String, _ error: any Error) {
        let message = "panops-shell: \(op) failed: \(error)\n"
        FileHandle.standardError.write(Data(message.utf8))
    }

    private func finishGenerationDone(meetingId: String, jobId: String? = nil, notesPath: String) {
        guard case .working(let currentMeetingId, _) = state, currentMeetingId == meetingId else { return }
        recordNotesProgressHeartbeat()
        pollingTask?.cancel()
        pollingTask = nil
        if let jobId {
            Task { await eventStream.unregisterCallback(jobId: jobId) }
        }
        notesProgress = nil
        notesLastProgressAt = nil
        notesGenMeetingId = nil
        // Signal the open meeting workspace to re-read notes.json from disk.
        notesReloadTick += 1
        state = .done(notesPath: notesPath)
        Task { await refreshMeetings() }
    }

    private func finishGenerationError(meetingId: String, jobId: String? = nil, kind: String, message: String) {
        guard case .working(let currentMeetingId, _) = state, currentMeetingId == meetingId else { return }
        recordNotesProgressHeartbeat()
        pollingTask?.cancel()
        pollingTask = nil
        if let jobId {
            Task { await eventStream.unregisterCallback(jobId: jobId) }
        }
        notesProgress = nil
        notesLastProgressAt = nil
        state = .error(kind: kind, message: message)
    }

    private func updateNotesProgress(_ progress: JobProgressEvent) {
        notesProgress = progress
        recordNotesProgressHeartbeat()
    }

    private func recordNotesProgressHeartbeat() {
        notesLastProgressAt = Date()
    }

    private func notesProgressStalledMessage() -> String {
        let minutes = Int(Self.progressSilenceTimeoutSeconds / 60)
        return "notes.generate stalled: no progress for \(minutes) minutes"
    }

    private func startPolling(meetingId: String, jobId: String? = nil) {
        // Fallback polling if WebSocket isn't available, and safety-net polling
        // if WebSocket disconnects before a terminal event is delivered.
        pollingTask?.cancel()
        notesLastProgressAt = Date()
        let client = self.client
        pollingTask = Task.detached { [weak self] in
            let mainActorRef = self
            let meeting: Meeting
            do {
                meeting = try await client.meetingGet(id: meetingId)
            } catch {
                Self.logFullError("meeting.get", error)
                await MainActor.run {
                    mainActorRef?.finishGenerationError(
                        meetingId: meetingId,
                        jobId: jobId,
                        kind: "internal",
                        message: "Lost contact with the engine."
                    )
                }
                return
            }
            let notesPath = (meeting.dirPath as NSString).appendingPathComponent("notes.md")
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                if FileManager.default.fileExists(atPath: notesPath) {
                    await MainActor.run {
                        mainActorRef?.finishGenerationDone(
                            meetingId: meetingId,
                            jobId: jobId,
                            notesPath: notesPath
                        )
                    }
                    return
                }
                let hasStalled = await MainActor.run { () -> Bool in
                    let lastProgressAt = mainActorRef?.notesLastProgressAt ?? Date()
                    return Date().timeIntervalSince(lastProgressAt) >= Self.progressSilenceTimeoutSeconds
                }
                if hasStalled {
                    await MainActor.run {
                        mainActorRef?.finishGenerationError(
                            meetingId: meetingId,
                            jobId: jobId,
                            kind: "timeout",
                            message: mainActorRef?.notesProgressStalledMessage()
                                ?? "notes.generate stalled: no progress"
                        )
                    }
                    return
                }
            }
        }
    }

    func reveal(_ path: String) {
        guard PathValidator.isUnderPanopsDataDir(path) else {
            Self.logFullError("reveal", NSError(domain: "PanopsShell", code: 1, userInfo: [NSLocalizedDescriptionKey: "refusing to reveal path outside panops data dir: \(path)"]))
            return
        }
        let url = URL(fileURLWithPath: path).standardizedFileURL
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    func reset() {
        pollingTask?.cancel()
        pollingTask = nil
        wsSubscriptionTask?.cancel()
        wsSubscriptionTask = nil
        Task { await eventStream.stop() }
        selectedMeetingId = nil
        activeRecordingMeetingId = nil
        activeRecordingDirPath = nil
        selectedMeeting = nil
        notesProgress = nil
        notesLastProgressAt = nil
        notesGenMeetingId = nil
        deferredNotesHint = nil
        saveStatus = .idle
        state = .idle(audio: nil)
    }

    /// Return from a browsed meeting detail to the audio-file generation flow.
    func startNewGenerationFlow() {
        selectedMeetingId = nil
        selectedMeeting = nil
        state = .idle(audio: nil)
    }

    func shutdown(engine: EngineProcess?) async {
        pollingTask?.cancel()
        wsSubscriptionTask?.cancel()
        await eventStream.stop()
        await client.disconnect()
        await engine?.stop()
    }
}
