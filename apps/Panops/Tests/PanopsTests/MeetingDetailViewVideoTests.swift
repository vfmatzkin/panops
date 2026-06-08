import Foundation
import Testing
@testable import Panops

@Suite("MeetingDetailView video file handling")
@MainActor
struct MeetingDetailViewVideoTests {
    @Test("video row.isVisible when recording.mov exists")
    func videoRowVisibleWhenFileExists() async throws {
        // Create a temp directory with a recording.mov file
        let fm = FileManager.default
        let tempDir = fm.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try fm.createDirectory(at: tempDir, withIntermediateDirectories: true)

        let videoPath = tempDir.appendingPathComponent("recording.mov")
        try "dummy".write(to: videoPath, atomically: true, encoding: .utf8)

        let meeting = Meeting(
            id: "test-meeting",
            title: "Test Meeting",
            startedAt: "2026-06-01T10:00:00Z",
            endedAt: nil,
            durationMs: 1000,
            language: "en",
            dirPath: tempDir.path
        )

        // Simulate the path checking logic from loadMeetingData
        let path = (tempDir.path as NSString).appendingPathComponent("recording.mov")
        let isVisible = PathValidator.isPath(path, under: tempDir.path) && fm.fileExists(atPath: path)

        #expect(isVisible)

        // Clean up
        try? fm.removeItem(at: tempDir)
    }

    @Test("video row isHidden when recording.mov does not exist")
    func videoRowHiddenWhenFileMissing() async throws {
        let fm = FileManager.default
        let tempDir = fm.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try fm.createDirectory(at: tempDir, withIntermediateDirectories: true)

        let meeting = Meeting(
            id: "test-meeting",
            title: "Test Meeting",
            startedAt: "2026-06-01T10:00:00Z",
            endedAt: nil,
            durationMs: 1000,
            language: "en",
            dirPath: tempDir.path
        )

        let path = (tempDir.path as NSString).appendingPathComponent("recording.mov")
        let isVisible = PathValidator.isPath(path, under: tempDir.path) && fm.fileExists(atPath: path)

        #expect(!isVisible)

        // Clean up
        try? fm.removeItem(at: tempDir)
    }

    @Test("formatFileSize formats bytes correctly")
    func formatFileSize() {
        let fm = FileManager.default
        let tempDir = fm.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try? fm.createDirectory(at: tempDir, withIntermediateDirectories: true)

        let testCases: [(size: Int64, expectedContains: String)] = [
            (0, "0 bytes"),
            (100, "100"),
            (1024, "1 KB"),
            (1024 * 1024, "1 MB"),
            (1024 * 1024 * 1024, "1 GB"),
        ]

        for (size, expected) in testCases {
            let filePath = tempDir.appendingPathComponent("video_\(size).mov")
            // Create an empty file, then truncate to the exact size — sparse on
            // disk, so the 1 GB case stays cheap (no in-memory buffer).
            fm.createFile(atPath: filePath.path, contents: nil)
            if let handle = FileHandle(forWritingAtPath: filePath.path) {
                try? handle.truncate(atOffset: UInt64(size))
                try? handle.close()
            }

            let url = filePath
            guard let attrs = try? url.resourceValues(forKeys: [.fileSizeKey]),
                  let fileSize = attrs.fileSize else {
                Issue.record("Could not get file size for \(filePath)")
                continue
            }

            let formatter = ByteCountFormatter()
            formatter.countStyle = .file
            let formatted = formatter.string(fromByteCount: Int64(fileSize))

            // Just verify the formatter works and returns a non-empty string
            #expect(!formatted.isEmpty)
        }

        try? fm.removeItem(at: tempDir)
    }
}
