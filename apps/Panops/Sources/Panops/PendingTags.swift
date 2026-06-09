import Foundation

/// Compute the subset of LLM-proposed tag names that haven't been assigned to
/// the meeting yet. The comparison is case-insensitive so "Swift" and "swift"
/// collapse to the same tag (the engine's `tag.create` is idempotent on name,
/// but we still want to avoid showing a suggestion the user already accepted).
///
/// - Parameters:
///   - proposed: Tag names from `notes.json` frontmatter (LLM output).
///   - assigned: Tag names already attached to the meeting (resolved from IDs).
/// - Returns: Proposed names not present in `assigned`, preserving the order
///   the LLM produced them.
func pendingTags(proposed: [String], assigned: [String]) -> [String] {
    let assignedLower = Set(assigned.map { $0.lowercased() })
    return proposed.filter { !assignedLower.contains($0.lowercased()) }
}
