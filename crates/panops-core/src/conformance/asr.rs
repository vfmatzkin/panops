use std::path::Path;

use crate::asr::AsrProvider;
use crate::wer::wer;

/// Conformance fixture metadata. Kept as a single source of truth so the
/// fixture set, expected languages, and WER policy stay in sync.
///
/// `wer_max = None` means no WER assertion runs for this fixture, by spec:
/// see slice 02 design (`mixed_60s`: auto-detect transcript too unstable to
/// gate on) and slice 03 design (`multi_speaker_60s`: multi-voice TTS pushes
/// WER too high to gate). Single-voice fixtures keep a tight cap.
struct FixtureMeta {
    name: &'static str,
    expected_languages: &'static [&'static str],
    wer_max: Option<f32>,
}

const FIXTURES: &[FixtureMeta] = &[
    FixtureMeta {
        name: "en_30s",
        expected_languages: &["en"],
        wer_max: Some(0.20),
    },
    FixtureMeta {
        name: "es_30s",
        expected_languages: &["es"],
        wer_max: Some(0.20),
    },
    FixtureMeta {
        name: "mixed_60s",
        expected_languages: &["en", "es"],
        wer_max: None,
    },
    FixtureMeta {
        name: "multi_speaker_60s",
        expected_languages: &["en"],
        wer_max: None,
    },
];

pub fn run_suite<P: AsrProvider>(provider: &P, fixtures_dir: &Path) {
    for meta in FIXTURES {
        run_one(provider, fixtures_dir, meta);
    }
}

fn run_one<P: AsrProvider>(provider: &P, fixtures_dir: &Path, meta: &FixtureMeta) {
    let name = meta.name;
    let audio = fixtures_dir.join("audio").join(format!("{name}.wav"));
    let transcript_path = fixtures_dir
        .join("audio")
        .join(format!("{name}.transcript.txt"));

    let result = provider
        .transcribe_full(&audio, None)
        .unwrap_or_else(|e| panic!("[{name}] transcribe_full failed: {e}"));

    assert!(!result.segments.is_empty(), "[{name}] no segments");
    let total_audio_ms = result.audio_duration_ms;
    let mut prev_end = 0_u64;
    for (i, seg) in result.segments.iter().enumerate() {
        assert!(seg.start_ms <= seg.end_ms, "[{name}] seg[{i}] start>end");
        assert!(
            seg.end_ms <= total_audio_ms + 100,
            "[{name}] seg[{i}] end {} > audio {} + 100",
            seg.end_ms,
            total_audio_ms
        );
        assert!(
            seg.start_ms >= prev_end,
            "[{name}] seg[{i}] overlaps prev (start {} < prev_end {})",
            seg.start_ms,
            prev_end
        );
        prev_end = seg.end_ms;
    }

    let langs: Vec<&str> = result
        .segments
        .iter()
        .filter_map(|s| s.language_detected.as_deref())
        .collect();
    assert!(!langs.is_empty(), "[{name}] no language_detected populated");

    let any_match = langs.iter().any(|l| meta.expected_languages.contains(l));
    assert!(
        any_match,
        "[{name}] expected one of {expected:?}, got {langs:?}",
        expected = meta.expected_languages
    );

    if let (false, Some(wer_max)) = (provider.is_fake(), meta.wer_max) {
        let ground_truth = std::fs::read_to_string(&transcript_path)
            .unwrap_or_else(|e| panic!("[{name}] read transcript: {e}"));
        let hypothesis = result
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let wer_value = wer(&ground_truth, &hypothesis);
        assert!(
            wer_value <= wer_max,
            "[{name}] WER {wer_value:.3} > {wer_max}\n  gt: {ground_truth:?}\n  hy: {hypothesis:?}"
        );
    }
}
