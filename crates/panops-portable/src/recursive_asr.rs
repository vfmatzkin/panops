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

/// Force-split threshold: in auto-detect mode, any merged region longer
/// than this is bisected at its midpoint even if all segments come back
/// high-confidence. Mirrors Whisper's native ~30s decoder context — once a
/// region exceeds it, the model commits to ONE language for the whole
/// call (the slice 07 limitation). Slice 08 smoke showed that on
/// bilingual hallucination (Spanish audio transcribed as English),
/// per-token confidence stays uniformly above any usable threshold, so
/// `Segment.confidence` cannot trigger recursion on its own for the
/// continuous code-switch case. Duration is the only signal that reliably
/// catches it. Cost in monolingual single-region recordings is bounded by
/// `MAX_DEPTH` (at most ~8 sub-region calls).
pub const MAX_AUTO_SPLIT_MS: u64 = 30_000;

/// Confidence-recursive transcription. See module docstring.
///
/// Public entry point: callers don't manage recursion depth; the
/// private `recurse` helper does. This keeps the implementation
/// detail (`depth: u32`) off the API surface.
pub fn transcribe_recursive(
    asr: &dyn AsrProvider,
    samples: &[f32],
    sample_rate: u32,
    region: SpeechRegion,
    language_hint: Option<&str>,
) -> Result<RegionResult, AsrError> {
    recurse(asr, samples, sample_rate, region, language_hint, 0)
}

fn recurse(
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

    // Find the lowest-confidence segment BEFORE we clone into
    // `offset_segments` — that way the data flow is "compute everything
    // from `raw_segments` first, then derive `offset_segments`", and a
    // future refactor changing the clone to `.into_iter()` can't
    // accidentally invalidate the worst-idx search.
    let worst_idx_and_conf = raw_segments
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.confidence.total_cmp(&b.confidence))
        .map(|(i, s)| (i, s.confidence, s.start_ms));

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

    let (_, worst_conf, worst_start_ms) =
        worst_idx_and_conf.expect("non-empty segments guaranteed above");

    // Algorithm step 7: decide whether to split.
    //
    // Two triggers, OR'd:
    //   (a) Worst per-segment confidence is below `THRESHOLD` — Whisper
    //       struggled somewhere in this region.
    //   (b) Region duration exceeds `MAX_AUTO_SPLIT_MS` AND we're in
    //       auto-detect mode — Whisper's decoder commits to one language
    //       on the first ~30s window, so any longer region risks losing
    //       a mid-region language switch even when token confidence
    //       stays uniformly high (bilingual hallucination case proven by
    //       slice 08 smoke: min token-probability mean was 0.83 on
    //       Spanish-audio-transcribed-as-English).
    let confidence_trigger = worst_conf < THRESHOLD;
    // Duration trigger only applies to real adapters. Test fakes
    // (deterministic transcript returners, canned-response fakes)
    // would produce duplicated output across the split halves because
    // they ignore the chunk and return the same segments every call.
    // Confidence-based recursion still applies to fakes — `LowConfidenceAsr`
    // and friends exercise it deliberately.
    let duration_trigger = duration_ms > MAX_AUTO_SPLIT_MS && !asr.is_fake();
    if !confidence_trigger && !duration_trigger {
        return Ok(RegionResult {
            segments: offset_segments,
            model,
        });
    }

    // Algorithm step 8: choose the split point.
    //
    // For the confidence trigger, split at the lowest-confidence
    // segment's start — that's where Whisper actually got confused.
    // For the duration-only trigger, split at the midpoint because the
    // worst-segment heuristic can't be trusted when Whisper is
    // confident in a wrong-language hallucination (token probabilities
    // stay uniformly high across the call).
    let raw_split_abs_ms = if confidence_trigger {
        worst_start_ms + region.start_ms
    } else {
        region.start_ms + duration_ms / 2
    };
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

    let left_result = recurse(asr, samples, sample_rate, left, None, depth + 1)?;
    let right_result = recurse(asr, samples, sample_rate, right, None, depth + 1)?;

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
    // Do the ms→sample-index math in u64 (saturating) and cast to
    // usize at the end. Matches the rest of the codebase's
    // u64-based conversions and avoids 32-bit-target truncation.
    let sr = u64::from(sample_rate);
    let start_u64 = region.start_ms.saturating_mul(sr).saturating_div(1_000);
    let end_u64 = region.end_ms.saturating_mul(sr).saturating_div(1_000);
    let len = samples.len();
    let start = (start_u64 as usize).min(len);
    let end = (end_u64 as usize).min(len).max(start);
    &samples[start..end]
}

fn offset_segment(mut seg: Segment, region_start_ms: u64, region_end_ms: u64) -> Segment {
    seg.start_ms = (seg.start_ms + region_start_ms).min(region_end_ms);
    seg.end_ms = (seg.end_ms + region_start_ms).min(region_end_ms);
    seg
}
