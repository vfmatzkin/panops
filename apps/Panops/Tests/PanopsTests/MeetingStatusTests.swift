import Foundation
import Testing
@testable import Panops

@MainActor
@Suite("Meeting status derivation")
struct MeetingStatusTests {
    private func makeViewModel() -> AppViewModel {
        // status(for:) never touches the client; a dummy socket path is fine.
        AppViewModel(client: IpcClient(socketPath: URL(fileURLWithPath: "/tmp/panops-test.sock")))
    }

    private func summary(
        id: String = "m-1",
        endedAt: String? = nil,
        hasNotes: Bool = false,
        durationMs: UInt64 = 1000
    ) -> MeetingSummary {
        MeetingSummary(
            id: id,
            title: "Meeting",
            startedAt: "2026-06-05T10:00:00Z",
            durationMs: durationMs,
            language: "en",
            endedAt: endedAt,
            hasNotes: hasNotes
        )
    }

    @Test("active recording wins")
    func recording() {
        let vm = makeViewModel()
        vm.activeRecordingMeetingId = "m-1"
        #expect(vm.status(for: summary()) == .recording)
    }

    @Test("in-flight notes generation reads as processing")
    func processing() {
        let vm = makeViewModel()
        vm.notesGenMeetingId = "m-1"
        vm.state = .working(meetingId: "m-1", audioName: "a.wav")
        #expect(vm.status(for: summary(hasNotes: false)) == .processing)
    }

    @Test("has_notes reads as ready")
    func ready() {
        let vm = makeViewModel()
        #expect(vm.status(for: summary(endedAt: "2026-06-05T10:30:00Z", hasNotes: true)) == .ready)
    }

    @Test("ended without notes reads as needs notes")
    func needsNotes() {
        let vm = makeViewModel()
        #expect(vm.status(for: summary(endedAt: "2026-06-05T10:30:00Z", hasNotes: false)) == .needsNotes)
    }

    @Test("open meeting with no notes and no duration reads as draft")
    func draft() {
        let vm = makeViewModel()
        #expect(vm.status(for: summary(endedAt: nil, hasNotes: false, durationMs: 0)) == .draft)
    }

    @Test("old payload lacking ended_at but with a duration reads as needs notes")
    func endedWithoutEndedAtFallsBackOnDuration() {
        let vm = makeViewModel()
        #expect(vm.status(for: summary(endedAt: nil, hasNotes: false, durationMs: 60_000)) == .needsNotes)
    }
}

@Suite("Meeting deletion guard")
struct MeetingDeletionGuardTests {
    @Test("recording and processing meetings are not deletable")
    func blockedStatuses() {
        #expect(MeetingStatus.recording.isDeletable == false)
        #expect(MeetingStatus.processing.isDeletable == false)
    }

    @Test("ready, needs-notes, and draft meetings are deletable")
    func allowedStatuses() {
        #expect(MeetingStatus.ready.isDeletable == true)
        #expect(MeetingStatus.needsNotes.isDeletable == true)
        #expect(MeetingStatus.draft.isDeletable == true)
    }
}
