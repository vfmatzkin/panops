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
    _depth: u32,
) -> Result<RegionResult, AsrError> {
    // D5: recursion is auto-mode only. If the caller passed a hint,
    // skip recursion and behave like the slice-07 single-call path.
    let chunk = slice_for_region(samples, sample_rate, &region);
    let transcript = asr.transcribe(chunk, sample_rate, language_hint)?;
    let segments: Vec<Segment> = transcript
        .segments
        .into_iter()
        .map(|seg| offset_segment(seg, region.start_ms, region.end_ms))
        .collect();
    let model = if segments.is_empty() {
        None
    } else {
        Some(transcript.model)
    };
    Ok(RegionResult { segments, model })
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
