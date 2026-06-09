import Foundation
import Testing

@testable import Panops

@Suite("PendingTags")
struct PendingTagsTests {
    // MARK: - Core subtraction

    @Test("returns all proposed when none are assigned")
    func allPendingWhenNoneAssigned() {
        let result = pendingTags(proposed: ["swift", "testing"], assigned: [])
        #expect(result == ["swift", "testing"])
    }

    @Test("returns empty when all proposed are already assigned")
    func emptyWhenAllAssigned() {
        let result = pendingTags(proposed: ["swift", "testing"], assigned: ["swift", "testing"])
        #expect(result == [])
    }

    @Test("returns only the unassigned subset")
    func partialOverlap() {
        let result = pendingTags(
            proposed: ["swift", "testing", "ios"],
            assigned: ["testing"]
        )
        #expect(result == ["swift", "ios"])
    }

    // MARK: - Case-insensitive matching

    @Test("case-insensitive: 'Swift' matches 'swift'")
    func caseInsensitiveMatch() {
        let result = pendingTags(proposed: ["Swift", "Rust"], assigned: ["swift"])
        #expect(result == ["Rust"])
    }

    @Test("case-insensitive: assigned 'TESTING' filters proposed 'testing'")
    func caseInsensitiveReverse() {
        let result = pendingTags(proposed: ["testing"], assigned: ["TESTING"])
        #expect(result == [])
    }

    // MARK: - Edge cases

    @Test("empty proposed returns empty regardless of assigned")
    func emptyProposed() {
        let result = pendingTags(proposed: [], assigned: ["swift"])
        #expect(result == [])
    }

    @Test("both empty returns empty")
    func bothEmpty() {
        let result = pendingTags(proposed: [], assigned: [])
        #expect(result == [])
    }

    @Test("preserves the LLM's original ordering")
    func preservesOrder() {
        let result = pendingTags(
            proposed: ["ios", "swift", "testing"],
            assigned: ["swift"]
        )
        #expect(result == ["ios", "testing"])
    }

    @Test("duplicate proposed names both surface if unassigned")
    func duplicateProposed() {
        // The LLM shouldn't produce duplicates, but the function doesn't
        // deduplicate — it filters against assigned. Caller can dedup if needed.
        let result = pendingTags(proposed: ["swift", "swift"], assigned: [])
        #expect(result == ["swift", "swift"])
    }

    @Test("whitespace matters: 'swift ' doesn't match 'swift'")
    func whitespaceSensitive() {
        // Case-insensitive but whitespace-sensitive — the engine normalizes
        // tag names on creation, so trailing spaces are a different tag.
        let result = pendingTags(proposed: ["swift "], assigned: ["swift"])
        #expect(result == ["swift "])
    }
}
