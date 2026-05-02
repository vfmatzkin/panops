//! Post-LLM verifier for section attribution. Enforces the speaker-attribution
//! rule that today only lives in the prompt (`prompts.rs:51-52`):
//!
//! > Speaker attribution rule (STRICT): never attribute a quote to a speaker_id
//! > that does not appear in the transcript.
//!
//! Verifies LLM-emitted `narrative_md` and `action_items[].owner` reference
//! only allowed `speaker_<u32>` IDs. Returns `VerifierReport::Ok` for valid
//! output and `VerifierReport::DisallowedSpeakers(set)` when at least one
//! reference falls outside the allowed set. The pipeline reacts by falling
//! back to a deterministic transcript dump for that section.
//!
//! Only ID-form references (`speaker_42`) are validated — invented names
//! ("Alex") aren't in scope; speaker-name resolution is a later slice (#21).
//! Frontmatter title verification is also out of scope for this pass.

use std::collections::HashSet;

use crate::notes::ir::ActionItem;

#[derive(Debug, PartialEq, Eq)]
pub enum VerifierReport {
    Ok,
    DisallowedSpeakers(HashSet<u32>),
}

/// Verifies a section's narrative + action items reference only allowed
/// `speaker_<u32>` IDs. Allowed IDs are typically `collect_speakers`'s set
/// derived from transcript segments.
pub fn verify_section_attribution(
    narrative_md: &str,
    action_items: &[ActionItem],
    allowed_speakers: &HashSet<u32>,
) -> VerifierReport {
    let mut disallowed: HashSet<u32> = HashSet::new();

    for id in scan_speaker_ids(narrative_md) {
        if !allowed_speakers.contains(&id) {
            disallowed.insert(id);
        }
    }

    for item in action_items {
        if let Some(owner) = item.owner.as_deref() {
            if let Some(id) = parse_speaker_id(owner) {
                if !allowed_speakers.contains(&id) {
                    disallowed.insert(id);
                }
            }
            // Non-`speaker_N` owner strings (e.g. "Alex") are not validated
            // here — see #21 for speaker-name resolution.
        }
    }

    if disallowed.is_empty() {
        VerifierReport::Ok
    } else {
        VerifierReport::DisallowedSpeakers(disallowed)
    }
}

/// Yields every `speaker_<u32>` ID found in the input, anchored on a word
/// boundary so substrings like `thespeaker_42` aren't matched. Tolerates
/// surrounding markdown (bold/italic markers, colons, parens) — those are
/// non-alphanumeric and treated as boundaries.
fn scan_speaker_ids(s: &str) -> impl Iterator<Item = u32> + '_ {
    s.match_indices("speaker_").filter_map(move |(start, _)| {
        // Require word boundary before the match: start-of-string or a
        // non-(alphanumeric|underscore) char.
        let preceded_by_word_char = s[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if preceded_by_word_char {
            return None;
        }
        let rest = &s[start + "speaker_".len()..];
        let digit_end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if digit_end == 0 {
            None
        } else {
            rest[..digit_end].parse::<u32>().ok()
        }
    })
}

/// Parses exactly `speaker_<u32>` (with optional surrounding whitespace).
/// Returns `None` if the input is shaped differently — those are not
/// considered ID references and are passed through verbatim.
fn parse_speaker_id(s: &str) -> Option<u32> {
    let trimmed = s.trim();
    let rest = trimmed.strip_prefix("speaker_")?;
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    rest.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(ids: &[u32]) -> HashSet<u32> {
        ids.iter().copied().collect()
    }

    fn item(desc: &str, owner: Option<&str>) -> ActionItem {
        ActionItem {
            description: desc.to_string(),
            owner: owner.map(String::from),
            due: None,
        }
    }

    #[test]
    fn ok_when_only_allowed_speakers_appear() {
        let narrative = "**speaker_0:** opened. **speaker_1:** asked.";
        let items = vec![item("draft", Some("speaker_1")), item("review", None)];
        assert_eq!(
            verify_section_attribution(narrative, &items, &allowed(&[0, 1])),
            VerifierReport::Ok
        );
    }

    #[test]
    fn flags_disallowed_speaker_in_narrative() {
        let narrative = "**speaker_99:** said something they did not say.";
        let report = verify_section_attribution(narrative, &[], &allowed(&[0, 1]));
        match report {
            VerifierReport::DisallowedSpeakers(s) => {
                assert!(s.contains(&99));
                assert_eq!(s.len(), 1);
            }
            VerifierReport::Ok => panic!("expected violation"),
        }
    }

    #[test]
    fn flags_disallowed_owner_in_action_item() {
        let items = vec![item("ship", Some("speaker_42"))];
        let report = verify_section_attribution("benign body", &items, &allowed(&[0, 1]));
        match report {
            VerifierReport::DisallowedSpeakers(s) => assert!(s.contains(&42)),
            VerifierReport::Ok => panic!("expected violation"),
        }
    }

    #[test]
    fn passes_through_non_id_owner_strings() {
        // "Alex" is not a speaker_ID form; verifier doesn't flag invented names.
        // Speaker-name resolution lives in a later slice (#21).
        let items = vec![item("draft", Some("Alex"))];
        assert_eq!(
            verify_section_attribution("clean body", &items, &allowed(&[0])),
            VerifierReport::Ok
        );
    }

    #[test]
    fn passive_voice_with_no_speaker_prefix_passes() {
        let narrative = "The agenda was reviewed and the team agreed on next steps.";
        assert_eq!(
            verify_section_attribution(narrative, &[], &allowed(&[0])),
            VerifierReport::Ok
        );
    }

    #[test]
    fn collects_multiple_disallowed_speakers() {
        let narrative = "speaker_99 and speaker_88 both spoke; speaker_0 nodded.";
        let report = verify_section_attribution(narrative, &[], &allowed(&[0]));
        match report {
            VerifierReport::DisallowedSpeakers(s) => {
                assert!(s.contains(&88));
                assert!(s.contains(&99));
                assert!(!s.contains(&0));
            }
            VerifierReport::Ok => panic!("expected violations"),
        }
    }

    #[test]
    fn ignores_non_digit_suffix_after_speaker_prefix() {
        // "speaker_" without trailing digits should not yield any ID.
        let narrative = "speaker_? unknown citation; speaker_0 was clear.";
        assert_eq!(
            verify_section_attribution(narrative, &[], &allowed(&[0])),
            VerifierReport::Ok
        );
    }

    #[test]
    fn word_boundary_skips_substring_matches() {
        // `thespeaker_99` and `notaspeaker_42` are *not* speaker references;
        // verifier should ignore them.
        let narrative = "thespeaker_99 ate the notaspeaker_42 cake. speaker_0 saw it.";
        assert_eq!(
            verify_section_attribution(narrative, &[], &allowed(&[0])),
            VerifierReport::Ok
        );
    }
}
