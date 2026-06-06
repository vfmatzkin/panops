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
        .task { await loadMeetingData() }
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

        // Validate path is under panops data directory before reading
        guard PathValidator.isUnderPanopsDataDir(dirPath) else {
            isLoading = false
            return
        }

        // Load transcript.json
        let transcriptPath = (dirPath as NSString).appendingPathComponent("transcript.json")
        if FileManager.default.fileExists(atPath: transcriptPath) {
            if let data = FileManager.default.contents(atPath: transcriptPath) {
                do {
                    transcript = try JSONDecoder().decode(Transcript.self, from: data)
                } catch {
                    // Keep nil on decode error
                }
            }
        }

        // Load notes.md
        let notesPath = (dirPath as NSString).appendingPathComponent("notes.md")
        if FileManager.default.fileExists(atPath: notesPath) {
            do {
                notesContent = try String(contentsOfFile: notesPath, encoding: .utf8)
            } catch {
                // Keep nil on error
            }
        }

        // Enumerate screenshots/ directory
        let screenshotsPath = (dirPath as NSString).appendingPathComponent("screenshots")
        if FileManager.default.fileExists(atPath: screenshotsPath) {
            do {
                let contents = try FileManager.default.contentsOfDirectory(atPath: screenshotsPath)
                screenshots = contents
                    .filter { $0.hasSuffix(".png") || $0.hasSuffix(".jpg") || $0.hasSuffix(".jpeg") }
                    .map { URL(fileURLWithPath: (screenshotsPath as NSString).appendingPathComponent($0)) }
                    .sorted { $0.lastPathComponent < $1.lastPathComponent }
            } catch {
                // Keep nil on error
            }
        }

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
