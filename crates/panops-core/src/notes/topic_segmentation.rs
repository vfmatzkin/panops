//! Deterministic rule-based topic segmentation.
//!
//! Opens a new section on any silence gap above `topic_gap_ms`. Merges
//! sections shorter than `min_section_ms` into the previous one.

use crate::Segment;

#[derive(Debug, Clone, Copy)]
pub struct TopicSegmentationConfig {
    pub topic_gap_ms: u64,
    pub min_section_ms: u64,
}

impl Default for TopicSegmentationConfig {
    fn default() -> Self {
        Self {
            topic_gap_ms: 8000,
            min_section_ms: 30_000,
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
        if gap > cfg.topic_gap_ms {
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
}
