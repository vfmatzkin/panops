use std::collections::HashSet;
use std::path::Path;

use crate::diar::Diarizer;

const FIXTURE: &str = "multi_speaker_60s";

pub fn run_suite<D: Diarizer>(provider: &D, fixtures_dir: &Path) {
    let audio = fixtures_dir.join("audio").join(format!("{FIXTURE}.wav"));
    let turns = provider
        .diarize(&audio)
        .unwrap_or_else(|e| panic!("[{FIXTURE}] diarize failed: {e}"));

    assert!(!turns.is_empty(), "[{FIXTURE}] no turns returned");

    let speakers: HashSet<u32> = turns.iter().map(|t| t.speaker_id).collect();
    assert!(
        speakers.len() >= 2,
        "[{FIXTURE}] expected >=2 distinct speakers, got {}",
        speakers.len()
    );

    let mut prev_end = 0_u64;
    for (i, t) in turns.iter().enumerate() {
        assert!(t.start_ms <= t.end_ms, "[{FIXTURE}] turn[{i}] start>end");
        assert!(
            t.start_ms >= prev_end,
            "[{FIXTURE}] turn[{i}] overlaps prev (start {} < prev_end {})",
            t.start_ms,
            prev_end
        );
        prev_end = t.end_ms;
    }

    let covered: u64 = turns
        .iter()
        .map(|t| t.end_ms.saturating_sub(t.start_ms))
        .sum();
    // TTS audio has long inter-sentence pauses that pyannote treats as non-speech.
    // Observed coverage ~24s (40%) on the sherpa-pyannote pipeline. Threshold at
    // 20000ms (33%) to pass real audio while still catching a fully-broken adapter.
    assert!(
        covered >= 20_000,
        "[{FIXTURE}] coverage {covered} ms < 20000 (33% of 60000); TTS audio floor"
    );

    if !provider.is_fake() {
        let s_first = dominant_speaker_in_window(&turns, 0, 20_000)
            .expect("[multi_speaker_60s] no dominant speaker in 0-20s");
        let s_third = dominant_speaker_in_window(&turns, 40_000, 60_000)
            .expect("[multi_speaker_60s] no dominant speaker in 40-60s");
        assert_eq!(
            s_first, s_third,
            "[{FIXTURE}] speaker re-identification failed: 0-20s={s_first}, 40-60s={s_third}"
        );
    }
}

fn dominant_speaker_in_window(
    turns: &[crate::diar::SpeakerTurn],
    start_ms: u64,
    end_ms: u64,
) -> Option<u32> {
    let mut totals: std::collections::HashMap<u32, u64> = Default::default();
    for t in turns {
        let lo = start_ms.max(t.start_ms);
        let hi = end_ms.min(t.end_ms);
        if hi > lo {
            *totals.entry(t.speaker_id).or_default() += hi - lo;
        }
    }
    totals.into_iter().max_by_key(|&(_, v)| v).map(|(k, _)| k)
}
