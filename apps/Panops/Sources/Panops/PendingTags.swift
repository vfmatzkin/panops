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

/// Resolve the tag names assigned to a meeting, given the meeting summaries and
/// the tag catalog.
///
/// `meetings` MUST be the full, unfiltered list (`allMeetings`) — not a
/// sidebar-filtered one. The detail pane shows `selectedMeeting` regardless of
/// the active filter, so resolving against a filtered list drops the open
/// meeting's tags whenever the filter excludes it: the assigned chips vanish
/// AND `pendingTags` (proposed minus assigned) then re-suggests names the user
/// already accepted.
///
/// - Parameters:
///   - meetingId: The open (displayed) meeting's id, or nil.
///   - meetings: The full, unfiltered meeting summaries.
///   - tags: The tag catalog (id → name).
/// - Returns: Assigned tag names, in `tags` order. Empty when the meeting is
///   nil or absent from `meetings`.
func resolveAssignedTagNames(
    forMeeting meetingId: String?,
    in meetings: [MeetingSummary],
    tags: [Tag]
) -> [String] {
    guard let meetingId,
          let summary = meetings.first(where: { $0.id == meetingId }) else {
        return []
    }
    let ids = Set(summary.tags)
    return tags.filter { ids.contains($0.id) }.map(\.name)
}
