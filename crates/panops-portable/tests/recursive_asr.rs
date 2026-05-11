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
