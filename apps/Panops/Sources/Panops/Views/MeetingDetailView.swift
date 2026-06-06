import SwiftUI

/// Container view for a selected meeting's content.
/// Reads transcript.json, notes.md, and enumerates screenshots/ directory.
/// Missing files → placeholders (expected when Slice 11 data doesn't exist).
struct MeetingDetailView: View {
    let meeting: Meeting
    let recordingController: MockRecordingController?
    @State private var transcript: Transcript?
    @State private var notesContent: String?
    @State private var screenshots: [URL]?
    @State private var isLoading = true

    init(meeting: Meeting, recordingController: MockRecordingController? = nil) {
        self.meeting = meeting
        self.recordingController = recordingController
    }

    var body: some View {
        VStack(spacing: 0) {
            // Header with meeting info
            headerView
            Divider()
            // Record bar (if controller provided)
            if let controller = recordingController {
                RecordBar(controller: controller)
                Divider()
            }
            // Content sections
            if isLoading {
                ProgressView("Loading…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        // Transcript section
                        SectionView(title: "Transcript") {
                            TranscriptView(transcript: transcript)
                        }
                        Divider()
                        // Notes section
                        SectionView(title: "Notes") {
                            NotesView(content: notesContent)
                        }
                        Divider()
                        // Screenshots section
                        SectionView(title: "Screenshots") {
                            ScreenshotsStripView(urls: screenshots)
                        }
                    }
                    .padding()
                }
            }
        }
        .task(id: meeting.id) { await loadMeetingData() }
    }

    private var headerView: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text(meeting.title).font(.headline)
                Text(meeting.startedAt).font(.caption).foregroundStyle(.secondary)
                if let duration = meeting.durationMs {
                    Text("Duration: \(formatDuration(duration))")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            Text(meeting.language.uppercased())
                .font(.caption)
                .padding(4)
                .background(Color.secondary.opacity(0.2))
                .cornerRadius(4)
        }
        .padding()
    }

    /// Load transcript.json, notes.md, and screenshots from meeting directory.
    private func loadMeetingData() async {
        // Reset all detail state at start to avoid stale data on meeting switch
        transcript = nil
        notesContent = nil
        screenshots = nil
        isLoading = true

        let dirPath = meeting.dirPath

        // Read + JSON-decode off the MainActor: transcript.json can be large for
        // long meetings, and a sync read+decode on the main thread hitches the UI
        // on every meeting selection (same hazard as ThumbnailView.loadImage).
        let loaded = await Task.detached(priority: .utility) { () -> (Transcript?, String?, [URL]?) in
            var t: Transcript?
            var notes: String?
            var shots: [URL]?

            let transcriptPath = (dirPath as NSString).appendingPathComponent("transcript.json")
            if PathValidator.isPath(transcriptPath, under: dirPath),
               FileManager.default.fileExists(atPath: transcriptPath),
               let data = FileManager.default.contents(atPath: transcriptPath) {
                t = try? JSONDecoder().decode(Transcript.self, from: data)
            }

            let notesPath = (dirPath as NSString).appendingPathComponent("notes.md")
            if PathValidator.isPath(notesPath, under: dirPath),
               FileManager.default.fileExists(atPath: notesPath) {
                notes = try? String(contentsOfFile: notesPath, encoding: .utf8)
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
            return (t, notes, shots)
        }.value

        // The .task(id:) cancels on meeting switch, but the detached read still
        // returns — bail so a slow load for the previous meeting can't overwrite
        // the newly-selected one's detail.
        guard !Task.isCancelled else { return }

        transcript = loaded.0
        notesContent = loaded.1
        screenshots = loaded.2
        isLoading = false
    }

    private func formatDuration(_ ms: UInt64) -> String {
        let totalSec = ms / 1000
        let min = totalSec / 60
        let sec = totalSec % 60
        return "\(min)m \(sec)s"
    }
}

/// Section wrapper with title header.
struct SectionView<Content: View>: View {
    let title: String
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.headline)
                .foregroundStyle(.primary)
            content()
        }
    }
}
