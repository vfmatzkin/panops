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

/// Combine the two capture tracks into one diarized segment list.
///
/// Mic-track segments are the local user → pinned to `speaker_id 0` ("You").
/// System-track segments are remote participants → each sherpa turn id is
/// offset by `+1` (past the reserved local id) and assigned by overlap via
/// [`merge_speaker_turns`]. The two lists are concatenated and stably sorted
/// by `start_ms`. Either input may be empty (single-track captures).
pub fn merge_two_track(
    mic_segments: Vec<Segment>,
    system_segments: Vec<Segment>,
    system_turns: &[SpeakerTurn],
) -> Vec<Segment> {
    let mut mic = mic_segments;
    for s in &mut mic {
        s.speaker_id = Some(0);
    }

    let offset_turns: Vec<SpeakerTurn> = system_turns
        .iter()
        .map(|t| SpeakerTurn {
            start_ms: t.start_ms,
            end_ms: t.end_ms,
            speaker_id: t.speaker_id + 1,
        })
        .collect();
    let system = merge_speaker_turns(system_segments, &offset_turns);

    let mut all = mic;
    all.extend(system);
    all.sort_by_key(|s| s.start_ms); // stable; mic keeps order vs system on ties
    all
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

    #[test]
    fn mic_segments_pinned_to_local_speaker_zero() {
        let mic = vec![seg(0, 1_000), seg(1_000, 2_000)];
        let out = merge_two_track(mic, vec![], &[]);
        assert!(out.iter().all(|s| s.speaker_id == Some(0)));
    }

    #[test]
    fn system_turns_offset_past_local() {
        let system = vec![seg(0, 1_000), seg(1_000, 2_000)];
        // sherpa speaker ids 0 and 1 → remote ids 1 and 2.
        let turns = vec![turn(0, 1_000, 0), turn(1_000, 2_000, 1)];
        let out = merge_two_track(vec![], system, &turns);
        assert_eq!(out[0].speaker_id, Some(1));
        assert_eq!(out[1].speaker_id, Some(2));
    }

    #[test]
    fn merged_output_is_timestamp_ordered() {
        let mic = vec![seg(500, 1_500)];
        let system = vec![seg(0, 400), seg(2_000, 2_500)];
        let turns = vec![turn(0, 3_000, 0)];
        let out = merge_two_track(mic, system, &turns);
        let starts: Vec<u64> = out.iter().map(|s| s.start_ms).collect();
        assert_eq!(starts, vec![0, 500, 2_000]);
        // mic segment is local 0; system segments are remote 1.
        assert_eq!(out[1].speaker_id, Some(0));
        assert_eq!(out[0].speaker_id, Some(1));
        assert_eq!(out[2].speaker_id, Some(1));
    }
}
