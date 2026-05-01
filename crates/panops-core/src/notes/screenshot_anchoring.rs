//! Maps screenshot timestamps to sections produced by topic segmentation.
//!
//! Half-open intervals (`time_range_ms.0 ≤ ms < time_range_ms.1`).
//! Out-of-range screenshots attach to the nearest section by midpoint
//! distance.

use super::ir::Screenshot;
use super::topic_segmentation::RawSection;

pub fn anchor_screenshots(
    sections: &[RawSection],
    screenshots: &[Screenshot],
) -> Vec<Vec<Screenshot>> {
    let mut out: Vec<Vec<Screenshot>> = sections.iter().map(|_| Vec::new()).collect();
    if sections.is_empty() {
        return out;
    }
    for shot in screenshots {
        let idx = section_for_timestamp(sections, shot.ms_since_start);
        out[idx].push(shot.clone());
    }
    out
}

fn section_for_timestamp(sections: &[RawSection], ms: u64) -> usize {
    for (i, s) in sections.iter().enumerate() {
        if ms >= s.time_range_ms.0 && ms < s.time_range_ms.1 {
            return i;
        }
    }
    let mut best = 0usize;
    let mut best_dist = u64::MAX;
    for (i, s) in sections.iter().enumerate() {
        let mid = s.time_range_ms.0 + (s.time_range_ms.1 - s.time_range_ms.0) / 2;
        let dist = mid.abs_diff(ms);
        if dist < best_dist {
            best_dist = dist;
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::ir::Screenshot;
    use crate::notes::topic_segmentation::RawSection;
    use std::path::PathBuf;

    fn shot(ms: u64) -> Screenshot {
        Screenshot {
            ms_since_start: ms,
            path: PathBuf::from(format!("/tmp/{ms}.jpg")),
            caption: None,
        }
    }

    fn raw(start: u64, end: u64) -> RawSection {
        RawSection {
            time_range_ms: (start, end),
            segment_indices: vec![],
        }
    }

    #[test]
    fn screenshot_anchors_in_the_section_containing_its_timestamp() {
        let secs = vec![raw(0, 30_000), raw(30_000, 60_000)];
        let shots = vec![shot(15_000), shot(45_000)];
        let assigned = anchor_screenshots(&secs, &shots);
        assert_eq!(assigned.len(), 2);
        assert_eq!(assigned[0].len(), 1);
        assert_eq!(assigned[0][0].ms_since_start, 15_000);
        assert_eq!(assigned[1][0].ms_since_start, 45_000);
    }

    #[test]
    fn screenshot_at_section_boundary_goes_to_later_section() {
        // Half-open intervals: ms == start belongs to that section,
        // ms == end belongs to the next.
        let secs = vec![raw(0, 30_000), raw(30_000, 60_000)];
        let shots = vec![shot(30_000)];
        let assigned = anchor_screenshots(&secs, &shots);
        assert_eq!(assigned[0].len(), 0);
        assert_eq!(assigned[1].len(), 1);
    }

    #[test]
    fn screenshot_past_last_section_attaches_to_nearest() {
        let secs = vec![raw(0, 30_000), raw(30_000, 60_000)];
        let shots = vec![shot(70_000)];
        let assigned = anchor_screenshots(&secs, &shots);
        assert_eq!(assigned[1].len(), 1);
        assert_eq!(assigned[1][0].ms_since_start, 70_000);
    }

    #[test]
    fn no_screenshots_yields_empty_per_section_lists() {
        let secs = vec![raw(0, 60_000)];
        let assigned = anchor_screenshots(&secs, &[]);
        assert_eq!(assigned, vec![Vec::<Screenshot>::new()]);
    }
}
