//! Deterministic rule-based topic segmentation.
//!
//! Opens a new section on either:
//!   - a silence gap above `topic_gap_ms`, OR
//!   - a speaker shift: the dominant speaker of the in-progress section
//!     is *not* the new segment's speaker, AND the new speaker has spoken
//!     less than `speaker_shift_threshold` of the section so far.
//!
//! Per slice 04 design spec (`docs/superpowers/specs/2026-05-01-slice-04-notes-generation-design.md:165`).
//! "Previous section" in the spec is read as the cumulative in-progress
//! section (the one currently being built), not the closed prior section.
//! Segments without `speaker_id` (None) don't contribute to either side of
//! the share calculation and never trigger a speaker-shift break.
//!
//! After segmentation, sections shorter than `min_section_ms` are merged
//! into the previous one.

use std::collections::HashMap;

use crate::Segment;

#[derive(Debug, Clone, Copy)]
pub struct TopicSegmentationConfig {
    pub topic_gap_ms: u64,
    pub min_section_ms: u64,
    pub speaker_shift_threshold: f32,
}

impl Default for TopicSegmentationConfig {
    fn default() -> Self {
        Self {
            topic_gap_ms: 8000,
            min_section_ms: 30_000,
            speaker_shift_threshold: 0.10,
        }
    }
}

/// A contiguous range of segments that share a topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSection {
    pub time_range_ms: (u64, u64),
    pub segment_indices: Vec<usize>,
}

pub fn segment_topics(segments: &[Segment], cfg: &TopicSegmentationConfig) -> Vec<RawSection> {
    if segments.is_empty() {
        return Vec::new();
    }

    let mut sections: Vec<RawSection> = Vec::new();
    let mut current = RawSection {
        time_range_ms: (segments[0].start_ms, segments[0].end_ms),
        segment_indices: vec![0],
    };

    for (i, seg) in segments.iter().enumerate().skip(1) {
        let prev_end = segments[i - 1].end_ms;
        let gap = seg.start_ms.saturating_sub(prev_end);
        let speaker_break = is_speaker_shift_break(
            segments,
            &current.segment_indices,
            seg.speaker_id,
            cfg.speaker_shift_threshold,
        );
        if gap > cfg.topic_gap_ms || speaker_break {
            sections.push(std::mem::replace(
                &mut current,
                RawSection {
                    time_range_ms: (seg.start_ms, seg.end_ms),
                    segment_indices: vec![i],
                },
            ));
        } else {
            current.time_range_ms.1 = seg.end_ms;
            current.segment_indices.push(i);
        }
    }
    sections.push(current);

    merge_short_sections(sections, cfg.min_section_ms)
}

/// Returns `true` if introducing `new_speaker` to the in-progress section
/// satisfies the speaker-shift rule: dominant speaker so far is different,
/// AND `new_speaker`'s share of the section so far is below `threshold`.
/// Segments without speakers (`None`) never trigger a break.
fn is_speaker_shift_break(
    segments: &[Segment],
    section_indices: &[usize],
    new_speaker: Option<u32>,
    threshold: f32,
) -> bool {
    let Some(new_id) = new_speaker else {
        return false;
    };
    let Some(dom) = dominant_speaker(segments, section_indices) else {
        // Section so far has no speakered segments; can't define dominance.
        return false;
    };
    if dom == new_id {
        return false;
    }
    speaker_share(segments, section_indices, new_id) < threshold
}

/// Dominant speaker of a section by summed segment duration. `None` if no
/// segment in the section has a `speaker_id`.
fn dominant_speaker(segments: &[Segment], indices: &[usize]) -> Option<u32> {
    let mut totals: HashMap<u32, u64> = HashMap::new();
    for &i in indices {
        let s = &segments[i];
        if let Some(id) = s.speaker_id {
            *totals.entry(id).or_default() += s.end_ms.saturating_sub(s.start_ms);
        }
    }
    totals
        .into_iter()
        .max_by_key(|&(_, ms)| ms)
        .map(|(id, _)| id)
}

/// Share of a section's *speakered* duration spoken by `who`. Returns 0.0
/// if the section has no speakered duration (denominator excludes None).
fn speaker_share(segments: &[Segment], indices: &[usize], who: u32) -> f32 {
    let mut total = 0u64;
    let mut who_total = 0u64;
    for &i in indices {
        let s = &segments[i];
        if let Some(id) = s.speaker_id {
            let dur = s.end_ms.saturating_sub(s.start_ms);
            total += dur;
            if id == who {
                who_total += dur;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        who_total as f32 / total as f32
    }
}

fn merge_short_sections(sections: Vec<RawSection>, min_ms: u64) -> Vec<RawSection> {
    let mut out: Vec<RawSection> = Vec::with_capacity(sections.len());
    for s in sections {
        let len = s.time_range_ms.1.saturating_sub(s.time_range_ms.0);
        if len < min_ms && !out.is_empty() {
            let prev = out.last_mut().unwrap();
            prev.time_range_ms.1 = s.time_range_ms.1;
            prev.segment_indices.extend(s.segment_indices);
        } else {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Segment;

    fn seg(start: u64, end: u64, speaker: Option<u32>, text: &str) -> Segment {
        Segment {
            start_ms: start,
            end_ms: end,
            text: text.into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            is_partial: false,
            speaker_id: speaker,
        }
    }

    #[test]
    fn single_continuous_segment_yields_one_section() {
        let segs = vec![seg(0, 60_000, Some(0), "hello world")];
        let secs = segment_topics(&segs, &TopicSegmentationConfig::default());
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].time_range_ms, (0, 60_000));
        assert_eq!(secs[0].segment_indices, vec![0]);
    }

    #[test]
    fn silence_gap_above_threshold_opens_new_section() {
        // Gap of 10s between [0..20s] and [30..60s] with default 8s threshold.
        let segs = vec![
            seg(0, 20_000, Some(0), "a"),
            seg(30_000, 60_000, Some(0), "b"),
        ];
        let secs = segment_topics(&segs, &TopicSegmentationConfig::default());
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].time_range_ms, (0, 20_000));
        assert_eq!(secs[1].time_range_ms, (30_000, 60_000));
    }

    #[test]
    fn silence_gap_below_threshold_keeps_one_section() {
        let segs = vec![
            seg(0, 20_000, Some(0), "a"),
            seg(22_000, 40_000, Some(0), "b"),
        ];
        let secs = segment_topics(&segs, &TopicSegmentationConfig::default());
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].time_range_ms, (0, 40_000));
    }

    #[test]
    fn small_section_below_min_length_merges_with_previous() {
        let cfg = TopicSegmentationConfig {
            topic_gap_ms: 1000,
            min_section_ms: 30_000,
            speaker_shift_threshold: 0.10,
        };
        let segs = vec![
            seg(0, 30_000, Some(0), "long"),
            seg(35_000, 38_000, Some(0), "short"),
        ];
        let secs = segment_topics(&segs, &cfg);
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].time_range_ms, (0, 38_000));
    }

    #[test]
    fn empty_input_yields_empty_sections() {
        let segs: Vec<Segment> = vec![];
        let secs = segment_topics(&segs, &TopicSegmentationConfig::default());
        assert!(secs.is_empty());
    }

    // --- Speaker-shift rule tests (#41) ---

    #[test]
    fn speaker_shift_below_threshold_opens_new_section() {
        // Section so far is 30s of speaker A; speaker B (0% share) takes the
        // floor. Default threshold is 10%, so a fresh voice triggers a break.
        // min_section_ms set to 1 so the rule's effect isn't masked by the
        // merge step folding the sliver back.
        let cfg = TopicSegmentationConfig {
            topic_gap_ms: 8000,
            min_section_ms: 1,
            speaker_shift_threshold: 0.10,
        };
        let segs = vec![
            seg(0, 30_000, Some(0), "a one"),
            seg(30_500, 35_000, Some(1), "b new"),
        ];
        let secs = segment_topics(&segs, &cfg);
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].time_range_ms, (0, 30_000));
        assert_eq!(secs[1].time_range_ms, (30_500, 35_000));
    }

    #[test]
    fn threshold_zero_disables_speaker_shift_breaks() {
        // With `speaker_shift_threshold = 0.0`, the strict-less-than check
        // `share < threshold` is never true — so the rule cannot fire regardless
        // of how the voices interleave. min_section relaxed so it doesn't mask.
        let cfg = TopicSegmentationConfig {
            topic_gap_ms: 8000,
            min_section_ms: 1,
            speaker_shift_threshold: 0.0,
        };
        let segs = vec![
            seg(0, 16_000, Some(0), "a"),
            seg(16_500, 19_500, Some(1), "b"),
            seg(20_000, 25_000, Some(0), "a back"),
        ];
        let secs = segment_topics(&segs, &cfg);
        assert_eq!(secs.len(), 1, "threshold=0 should disable speaker-shift");
        assert_eq!(secs[0].time_range_ms, (0, 25_000));
    }

    #[test]
    fn gap_and_speaker_shift_yield_single_break() {
        // Both rules trigger between segs[0] and segs[1]; we still want one
        // boundary, not two.
        let segs = vec![
            seg(0, 20_000, Some(0), "a"),
            seg(40_000, 70_000, Some(1), "b"),
        ];
        let secs = segment_topics(&segs, &TopicSegmentationConfig::default());
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].time_range_ms, (0, 20_000));
        assert_eq!(secs[1].time_range_ms, (40_000, 70_000));
    }

    #[test]
    fn min_section_merge_after_speaker_break_folds_short_section() {
        // Speaker shift opens a new section after 30s of A; B speaks for 5s.
        // 5s is below the 30s min_section_ms, so it merges into the prior.
        let segs = vec![
            seg(0, 30_000, Some(0), "a long"),
            seg(30_500, 35_500, Some(1), "b sliver"),
        ];
        let secs = segment_topics(&segs, &TopicSegmentationConfig::default());
        assert_eq!(secs.len(), 1, "5s sliver should merge back into prior 30s");
        assert_eq!(secs[0].time_range_ms, (0, 35_500));
    }

    #[test]
    fn share_exactly_at_threshold_does_not_break() {
        // The check is `share < threshold`. A speaker whose share is exactly
        // the threshold (e.g. 10%) should NOT trigger the rule — they are
        // *at* the boundary, not below it. Setup: section has A=9s, B=1s
        // already (B share=10%). Then B speaks again — should not break.
        // This is a degenerate setup because reaching this state requires
        // earlier breaks to be suppressed; we use threshold=0.0 to reach it,
        // then mid-flight bump expectation by checking with threshold=0.1
        // applied to a fresh seq.
        //
        // Easier path: confirm with synthetic indices via direct helper call.
        let segs = vec![
            seg(0, 9_000, Some(0), "a"),
            seg(9_000, 10_000, Some(1), "b"),
        ];
        let indices = vec![0, 1];
        // share of speaker 1 = 1000 / 10000 = 0.10 exactly.
        let share = speaker_share(&segs, &indices, 1);
        assert!((share - 0.10).abs() < 1e-6, "expected ~0.10, got {share}");
        // is_speaker_shift_break with threshold 0.10 and incoming speaker=1:
        // dom is 0, new is 1, share is 0.10. 0.10 < 0.10 is false → no break.
        assert!(!is_speaker_shift_break(&segs, &indices, Some(1), 0.10));
        // Strictly below (0.099) does break:
        assert!(is_speaker_shift_break(&segs, &indices, Some(1), 0.11));
    }

    #[test]
    fn no_speaker_break_when_segment_has_no_speaker_id() {
        // A segment with speaker_id = None never triggers a speaker-shift
        // break (we can't compare what we don't know).
        let segs = vec![
            seg(0, 30_000, Some(0), "a"),
            seg(30_500, 35_000, None, "no-speaker"),
        ];
        let cfg = TopicSegmentationConfig {
            topic_gap_ms: 8000,
            min_section_ms: 1,
            speaker_shift_threshold: 0.10,
        };
        let secs = segment_topics(&segs, &cfg);
        assert_eq!(secs.len(), 1, "None speaker shouldn't open a new section");
    }
}
