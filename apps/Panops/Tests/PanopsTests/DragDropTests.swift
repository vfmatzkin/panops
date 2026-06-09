import Foundation
import Testing

@testable import Panops

@Suite("MeetingDropTarget mapping")
struct DragDropTests {
    // MARK: - Equality / identity

    @Test("space targets compare by space id")
    func spaceTargetEquality() {
        #expect(MeetingDropTarget.space(spaceId: "s1") == MeetingDropTarget.space(spaceId: "s1"))
        #expect(MeetingDropTarget.space(spaceId: "s1") != MeetingDropTarget.space(spaceId: "s2"))
    }

    @Test("project targets compare by project id")
    func projectTargetEquality() {
        #expect(MeetingDropTarget.project(projectId: "p1") == MeetingDropTarget.project(projectId: "p1"))
        #expect(MeetingDropTarget.project(projectId: "p1") != MeetingDropTarget.project(projectId: "p2"))
    }

    @Test("tag targets compare by tag id")
    func tagTargetEquality() {
        #expect(MeetingDropTarget.tag(tagId: "t1") == MeetingDropTarget.tag(tagId: "t1"))
        #expect(MeetingDropTarget.tag(tagId: "t1") != MeetingDropTarget.tag(tagId: "t2"))
    }

    // MARK: - Case discrimination (drop-target → which assign call)

    @Test("space and project with the same id are distinct cases")
    func differentRowTypesAreDistinct() {
        // Guards against a future refactor that collapses the cases into a
        // single "id + kind" pair with a shared equality — the downstream
        // IPC call depends on the case, not just the id.
        let space = MeetingDropTarget.space(spaceId: "x")
        let project = MeetingDropTarget.project(projectId: "x")
        let tag = MeetingDropTarget.tag(tagId: "x")
        #expect(space != project)
        #expect(project != tag)
        #expect(space != tag)
    }

    // MARK: - Failure-message labels

    @Test("operationNoun is 'space' for space targets")
    func spaceOperationNoun() {
        #expect(MeetingDropTarget.space(spaceId: "s").operationNoun == "space")
    }

    @Test("operationNoun is 'project' for project targets")
    func projectOperationNoun() {
        #expect(MeetingDropTarget.project(projectId: "p").operationNoun == "project")
    }

    @Test("operationNoun is 'tag' for tag targets")
    func tagOperationNoun() {
        #expect(MeetingDropTarget.tag(tagId: "t").operationNoun == "tag")
    }

    // MARK: - Transferable payload

    @Test("MeetingDragPayload carries the meeting id")
    func payloadCarriesId() {
        let payload = MeetingDragPayload(meetingId: "m-42")
        #expect(payload.meetingId == "m-42")
        #expect(payload == MeetingDragPayload(meetingId: "m-42"))
        #expect(payload != MeetingDragPayload(meetingId: "m-99"))
    }
}
