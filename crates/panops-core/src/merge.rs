use crate::diar::SpeakerTurn;
use crate::segment::Segment;

/// Merge speaker turns into segments by overlap. Each segment gets the
/// `speaker_id` of the turn that overlaps it the most. Segments with
/// no overlapping turn are returned with `speaker_id = None`.
pub fn merge_speaker_turns(segments: Vec<Segment>, turns: &[SpeakerTurn]) -> Vec<Segment> {
    segments
        .into_iter()
        .map(|mut seg| {
            seg.speaker_id = dominant_speaker(seg.start_ms, seg.end_ms, turns);
            seg
        })
        .collect()
}

fn dominant_speaker(start_ms: u64, end_ms: u64, turns: &[SpeakerTurn]) -> Option<u32> {
    let mut best: Option<(u32, u64)> = None;
    for t in turns {
        let lo = start_ms.max(t.start_ms);
        let hi = end_ms.min(t.end_ms);
        if hi <= lo {
            continue;
        }
        let overlap = hi - lo;
        match best {
            Some((_, b)) if overlap <= b => {}
            _ => best = Some((t.speaker_id, overlap)),
        }
    }
    best.map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_ms: u64, end_ms: u64) -> Segment {
        Segment {
            start_ms,
            end_ms,
            text: String::new(),
            language_detected: None,
            confidence: 1.0,
            is_partial: false,
            speaker_id: None,
        }
    }

    fn turn(start_ms: u64, end_ms: u64, speaker_id: u32) -> SpeakerTurn {
        SpeakerTurn {
            start_ms,
            end_ms,
            speaker_id,
        }
    }

    #[test]
    fn segment_fully_inside_one_turn() {
        let segs = vec![seg(1_000, 2_000)];
        let turns = vec![turn(0, 5_000, 7)];
        let out = merge_speaker_turns(segs, &turns);
        assert_eq!(out[0].speaker_id, Some(7));
    }

    #[test]
    fn segment_spans_two_turns_picks_dominant() {
        let segs = vec![seg(0, 1_000)];
        let turns = vec![turn(0, 600, 0), turn(600, 5_000, 1)];
        let out = merge_speaker_turns(segs, &turns);
        assert_eq!(out[0].speaker_id, Some(0));
    }

    #[test]
    fn segment_with_no_overlapping_turn_is_none() {
        let segs = vec![seg(0, 1_000)];
        let turns = vec![turn(2_000, 3_000, 0)];
        let out = merge_speaker_turns(segs, &turns);
        assert_eq!(out[0].speaker_id, None);
    }

    #[test]
    fn empty_turns_leaves_speaker_none() {
        let segs = vec![seg(0, 1_000), seg(1_000, 2_000)];
        let out = merge_speaker_turns(segs, &[]);
        assert!(out.iter().all(|s| s.speaker_id.is_none()));
    }
}
