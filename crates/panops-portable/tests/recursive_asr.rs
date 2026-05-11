//! Unit tests for `panops_portable::recursive_asr::transcribe_recursive`
//! against the `LowConfidenceAsr` fake. No ML inference; pure logic.

use std::path::PathBuf;

use panops_core::conformance::fakes::LowConfidenceAsr;
use panops_core::vad::SpeechRegion;
use panops_core::{Segment, Transcript};
#[allow(unused_imports)]
use panops_portable::recursive_asr::{MAX_DEPTH, MIN_REGION_MS, transcribe_recursive};

fn segment(start_ms: u64, end_ms: u64, lang: &str, confidence: f32) -> Segment {
    Segment {
        start_ms,
        end_ms,
        text: format!("{lang} text"),
        language_detected: Some(lang.into()),
        confidence,
        is_partial: false,
        speaker_id: None,
    }
}

fn transcript(model: &str, segments: Vec<Segment>) -> Transcript {
    let last_end = segments.last().map(|s| s.end_ms).unwrap_or(0);
    Transcript {
        schema_version: Transcript::SCHEMA_VERSION,
        model: model.into(),
        audio_path: PathBuf::new(),
        audio_duration_ms: last_end,
        diarized: false,
        segments,
    }
}

#[test]
fn high_confidence_region_passes_through_without_recursion() {
    let sample_rate = 16_000_u32;
    let samples = vec![0.0_f32; 30 * sample_rate as usize]; // 30s of zeros
    let region = SpeechRegion {
        start_ms: 0,
        end_ms: 30_000,
    };

    let fake = LowConfidenceAsr::with_responses(vec![transcript(
        "fake-whisper-en",
        vec![segment(0, 30_000, "en", 0.9)],
    )]);

    let result =
        transcribe_recursive(&fake, &samples, sample_rate, region, None, 0).expect("recursive ok");

    assert_eq!(fake.call_count(), 1, "no recursion expected");
    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments[0].language_detected.as_deref(), Some("en"));
    assert_eq!(result.model.as_deref(), Some("fake-whisper-en"));
}

#[test]
fn low_confidence_region_splits_at_lowest_segment_and_yields_both_languages() {
    let sample_rate = 16_000_u32;
    let samples = vec![0.0_f32; 30 * sample_rate as usize]; // 30s
    let region = SpeechRegion {
        start_ms: 0,
        end_ms: 30_000,
    };

    let fake = LowConfidenceAsr::with_responses(vec![
        // Call 0: top-level. Two low-confidence segments. The second
        // (at 15_000ms) is the lowest-confidence; that's where the split
        // lands.
        transcript(
            "fake-bilingual-mixed",
            vec![
                segment(0, 15_000, "en", 0.4),
                segment(15_000, 30_000, "es", 0.3),
            ],
        ),
        // Call 1: left half [0, 15_000ms]. duration = 15s < 2 * MIN_REGION_MS
        // (=20s), so the recursion short-circuits and returns this verbatim.
        transcript("fake-whisper-en", vec![segment(0, 15_000, "en", 0.9)]),
        // Call 2: right half [15_000, 30_000ms]. Same short-circuit.
        // Note: local-time inside the right region is [0, 15_000]; the
        // recursive function offsets by region.start_ms back to absolute.
        transcript("fake-whisper-es", vec![segment(0, 15_000, "es", 0.9)]),
    ]);

    let result =
        transcribe_recursive(&fake, &samples, sample_rate, region, None, 0).expect("recursive ok");

    assert_eq!(fake.call_count(), 3, "expected exactly one split");
    let langs: Vec<&str> = result
        .segments
        .iter()
        .filter_map(|s| s.language_detected.as_deref())
        .collect();
    assert!(langs.contains(&"en"), "left-half lang missing: {langs:?}");
    assert!(langs.contains(&"es"), "right-half lang missing: {langs:?}");
    // Model propagation: first non-empty call (call 0 had segments).
    assert_eq!(result.model.as_deref(), Some("fake-bilingual-mixed"));
    // Segment times are absolute (right-half got offset by 15_000ms).
    assert!(result.segments.iter().any(|s| s.start_ms == 15_000));
}

#[test]
fn always_low_confidence_caps_at_max_depth() {
    let sample_rate = 16_000_u32;
    let total_ms: u64 = 160_000; // 160s — plenty to bisect MAX_DEPTH times.
    let samples = vec![0.0_f32; (total_ms as usize / 1000) * sample_rate as usize];
    let region = SpeechRegion {
        start_ms: 0,
        end_ms: total_ms,
    };

    // One canned response: a single low-confidence segment placed at
    // 50% of whatever region we're transcribing. The fake doesn't know
    // the region; it always returns the same transcript with local
    // start_ms = 0, which collapses to "split at the start of this
    // region" — the clamp inside transcribe_recursive keeps it >=
    // MIN_REGION_MS, so we bisect repeatedly until depth cap.
    let fake = LowConfidenceAsr::with_responses(vec![transcript(
        "fake-always-low",
        vec![segment(0, 30_000, "en", 0.2)],
    )]);

    let result =
        transcribe_recursive(&fake, &samples, sample_rate, region, None, 0).expect("recursive ok");

    // Upper bound: full binary tree at MAX_DEPTH. The clamp guarantees
    // the floor; below-floor short-circuit prunes branches earlier.
    let max_calls = (1_u32 << (MAX_DEPTH + 1)) - 1; // 2^(d+1) - 1
    assert!(
        fake.call_count() <= max_calls as usize,
        "call count {} exceeded max {} (recursion not bounded)",
        fake.call_count(),
        max_calls
    );
    // The recursion did return something (depth-cap path).
    assert!(
        !result.segments.is_empty(),
        "depth cap should still return segments"
    );
}
