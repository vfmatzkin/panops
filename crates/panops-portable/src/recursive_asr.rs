//! Confidence-recursive ASR orchestration. Splits long, low-confidence
//! speech regions at the lowest-confidence segment boundary and re-runs
//! ASR per half. Lives in `panops-portable` because it composes around
//! any `AsrProvider`; no model state; deterministic given inputs.
//!
//! Slice 08 design: `docs/superpowers/specs/2026-05-11-slice-08-confidence-recursion-design.md`.

use panops_core::Segment;
use panops_core::asr::{AsrError, AsrProvider};
use panops_core::vad::SpeechRegion;

/// Output of one recursive transcription. `segments` already have
/// absolute timestamps (offset by `region.start_ms`). `model` is the
/// model string from the first non-empty `asr.transcribe` call in the
/// recursion (preserves slice-07 model-provenance behavior).
pub struct RegionResult {
    pub segments: Vec<Segment>,
    pub model: Option<String>,
}

/// Confidence threshold below which we recurse. Tied to
/// `Segment.confidence` (probability-space mean of token probabilities).
pub const THRESHOLD: f32 = 0.5;

/// Minimum region duration. Splitting below this is unreliable for
/// Whisper's language detection.
pub const MIN_REGION_MS: u64 = 10_000;

/// Recursion depth cap. With `MIN_REGION_MS = 10s` this bounds the
/// worst-case fan-out at 2^MAX_DEPTH = 8 leaves per top-level region.
pub const MAX_DEPTH: u32 = 3;

/// Confidence-recursive transcription. See module docstring.
pub fn transcribe_recursive(
    asr: &dyn AsrProvider,
    samples: &[f32],
    sample_rate: u32,
    region: SpeechRegion,
    language_hint: Option<&str>,
    depth: u32,
) -> Result<RegionResult, AsrError> {
    let chunk = slice_for_region(samples, sample_rate, &region);
    let transcript = asr.transcribe(chunk, sample_rate, language_hint)?;
    let raw_segments = transcript.segments;
    let model = if raw_segments.is_empty() {
        None
    } else {
        Some(transcript.model)
    };
    let offset_segments: Vec<Segment> = raw_segments
        .iter()
        .cloned()
        .map(|seg| offset_segment(seg, region.start_ms, region.end_ms))
        .collect();

    // D5: language hint forces non-recursive behavior.
    if language_hint.is_some() {
        return Ok(RegionResult {
            segments: offset_segments,
            model,
        });
    }

    // Algorithm step 5: short-circuit on empty, depth cap, or below-floor regions.
    let duration_ms = region.end_ms.saturating_sub(region.start_ms);
    if offset_segments.is_empty() || depth >= MAX_DEPTH || duration_ms < 2 * MIN_REGION_MS {
        return Ok(RegionResult {
            segments: offset_segments,
            model,
        });
    }

    // Algorithm step 6: locate the lowest-confidence segment.
    let worst_idx = raw_segments
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.confidence.total_cmp(&b.confidence))
        .map(|(i, _)| i)
        .expect("non-empty segments guaranteed above");
    let worst_conf = raw_segments[worst_idx].confidence;

    // Algorithm step 7: if even the worst is confident, accept the result.
    if worst_conf >= THRESHOLD {
        return Ok(RegionResult {
            segments: offset_segments,
            model,
        });
    }

    // Algorithm step 8: clamp the split point so neither half drops
    // below MIN_REGION_MS.
    let raw_split_abs_ms = raw_segments[worst_idx].start_ms + region.start_ms;
    let split_min = region.start_ms + MIN_REGION_MS;
    let split_max = region.end_ms.saturating_sub(MIN_REGION_MS);
    let split_abs_ms = raw_split_abs_ms.clamp(split_min, split_max);

    let left = SpeechRegion {
        start_ms: region.start_ms,
        end_ms: split_abs_ms,
    };
    let right = SpeechRegion {
        start_ms: split_abs_ms,
        end_ms: region.end_ms,
    };

    let left_result = transcribe_recursive(asr, samples, sample_rate, left, None, depth + 1)?;
    let right_result = transcribe_recursive(asr, samples, sample_rate, right, None, depth + 1)?;

    let mut segments = left_result.segments;
    segments.extend(right_result.segments);

    // Model propagation: the top-level call's model wins; only fall
    // back to a recursive frame's model if the top-level returned no
    // segments (very rare; defensive).
    let final_model = model.or(left_result.model).or(right_result.model);

    Ok(RegionResult {
        segments,
        model: final_model,
    })
}

fn slice_for_region<'a>(samples: &'a [f32], sample_rate: u32, region: &SpeechRegion) -> &'a [f32] {
    let start = ((region.start_ms as usize) * sample_rate as usize) / 1000;
    let end = ((region.end_ms as usize) * sample_rate as usize) / 1000;
    let start = start.min(samples.len());
    let end = end.min(samples.len()).max(start);
    &samples[start..end]
}

fn offset_segment(mut seg: Segment, region_start_ms: u64, region_end_ms: u64) -> Segment {
    seg.start_ms = (seg.start_ms + region_start_ms).min(region_end_ms);
    seg.end_ms = (seg.end_ms + region_start_ms).min(region_end_ms);
    seg
}
