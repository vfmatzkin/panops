import Foundation
import Testing
@testable import Panops

@Suite("PathValidator")
struct PathValidatorTests {
    @Test("accepts panops data root and descendants")
    func acceptsRootAndDescendants() {
        let root = PathValidator.panopsDataRoot

        #expect(PathValidator.isUnderPanopsDataDir(root))
        #expect(PathValidator.isUnderPanopsDataDir(root + "/meetings/meeting-1/notes.md"))
    }

    @Test("rejects sibling with panops prefix")
    func rejectsSiblingWithPanopsPrefix() {
        let root = PathValidator.panopsDataRoot
        let maliciousSibling = root + "-malicious/meetings/meeting-1/notes.md"

        #expect(!PathValidator.isUnderPanopsDataDir(maliciousSibling))
    }
}
