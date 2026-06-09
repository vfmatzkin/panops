import Foundation
import Testing

@testable import Panops

@Suite("AssignedTags")
struct AssignedTagsTests {
    // `MeetingSummary` only has a decoder init, so build fixtures from JSON —
    // exercising the real wire-decode path the app uses.
    private func summary(id: String, tagIds: [String]) -> MeetingSummary {
        let tagsJson = tagIds.map { "\"\($0)\"" }.joined(separator: ",")
        let json = """
        {"id":"\(id)","title":"T","started_at":"2026-01-01T00:00:00Z",\
        "duration_ms":0,"tags":[\(tagsJson)]}
        """
        return try! JSONDecoder().decode(MeetingSummary.self, from: Data(json.utf8))
    }

    private let catalog = [
        Tag(id: "t-swift", name: "Swift"),
        Tag(id: "t-rust", name: "Rust"),
        Tag(id: "t-ios", name: "iOS"),
    ]

    // MARK: - Resolution

    @Test("resolves assigned names for the open meeting, in catalog order")
    func resolvesNames() {
        let meetings = [summary(id: "m-1", tagIds: ["t-ios", "t-swift"])]
        let names = resolveAssignedTagNames(forMeeting: "m-1", in: meetings, tags: catalog)
        // Catalog order (Swift before iOS), not the meeting's tag-id order.
        #expect(names == ["Swift", "iOS"])
    }

    @Test("nil meeting id resolves to no assigned names")
    func nilMeeting() {
        let meetings = [summary(id: "m-1", tagIds: ["t-swift"])]
        #expect(resolveAssignedTagNames(forMeeting: nil, in: meetings, tags: catalog) == [])
    }

    @Test("meeting absent from the list resolves to no assigned names")
    func absentMeeting() {
        // The filtered-out failure mode: when the lookup list (e.g. a
        // sidebar-filtered `vm.meetings`) doesn't contain the open meeting, the
        // resolver yields nothing. The fix avoids this by passing the unfiltered
        // `vm.allMeetings`, which always contains the open meeting.
        let filteredOut: [MeetingSummary] = []
        #expect(resolveAssignedTagNames(forMeeting: "m-1", in: filteredOut, tags: catalog) == [])
    }

    @Test("unknown tag ids are dropped, not surfaced as empty names")
    func unknownTagIds() {
        let meetings = [summary(id: "m-1", tagIds: ["t-swift", "t-gone"])]
        let names = resolveAssignedTagNames(forMeeting: "m-1", in: meetings, tags: catalog)
        #expect(names == ["Swift"])
    }

    // MARK: - Interaction with pendingTags (the bug this fix prevents)

    @Test("resolved-from-full-list assignments hide already-accepted suggestions")
    func pendingHidesAcceptedWhenResolvedFromFullList() {
        // The open meeting is present in the (unfiltered) list with "Swift"
        // assigned. pendingTags must subtract it from the LLM's proposals.
        let allMeetings = [summary(id: "m-1", tagIds: ["t-swift"])]
        let assigned = resolveAssignedTagNames(forMeeting: "m-1", in: allMeetings, tags: catalog)
        let proposed = ["Swift", "Rust"]
        let pending = pendingTags(proposed: proposed, assigned: assigned)
        #expect(assigned == ["Swift"])
        #expect(pending == ["Rust"]) // "Swift" already accepted → not re-suggested
    }

    @Test("resolving from a filtered list re-suggests already-accepted tags")
    func pendingRegressesWhenMeetingFilteredOut() {
        // Documents the bug: when the open meeting is missing from the lookup
        // list, assignments resolve empty and pendingTags re-suggests the tag
        // the user already accepted ("Swift").
        let filteredOut: [MeetingSummary] = []
        let assigned = resolveAssignedTagNames(forMeeting: "m-1", in: filteredOut, tags: catalog)
        let pending = pendingTags(proposed: ["Swift", "Rust"], assigned: assigned)
        #expect(assigned == [])
        #expect(pending == ["Swift", "Rust"]) // already-accepted "Swift" wrongly returns
    }
}
