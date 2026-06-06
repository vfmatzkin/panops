import Foundation
import Testing
@testable import Panops

@Suite("PathValidator")
struct PathValidatorTests {
    @Test("accepts selected meeting dir and descendants outside app data")
    func acceptsSelectedMeetingDirOutsideAppData() {
        let meetingDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("panops-registered-meeting-\(UUID().uuidString)")
            .standardizedFileURL
            .path

        #expect(PathValidator.isPath(meetingDir, under: meetingDir))
        #expect(PathValidator.isPath(meetingDir + "/transcript.json", under: meetingDir))
        #expect(PathValidator.isPath(meetingDir + "/screenshots/frame-001.png", under: meetingDir))
    }

    @Test("rejects sibling with selected meeting dir prefix")
    func rejectsSiblingWithSelectedMeetingDirPrefix() {
        let meetingDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("panops-registered-meeting-\(UUID().uuidString)")
            .standardizedFileURL
            .path
        let maliciousSibling = meetingDir + "-evil/screenshots/frame-001.png"

        #expect(!PathValidator.isPath(maliciousSibling, under: meetingDir))
    }
}
