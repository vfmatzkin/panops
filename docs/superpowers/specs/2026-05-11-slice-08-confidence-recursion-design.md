# Slice 08 — Confidence-Recursive ASR for Continuous-Speech Bilingual Code-Switching

**Status:** Locked design with one post-implementation amendment (see below).
**Date:** 2026-05-11
**Author:** Franco Matzkin (with Claude as brainstorm partner)
**Predecessor:** [slice 07 design](2026-05-09-slice-07-vad-multilingual-asr-design.md)
**North-star tie-in:** Closes the "multilingual day 1, no language toggle" gap on continuous-speech bilingual recordings (the path slice 07 left open at D6 and Open Question #1).

## Amendment — 2026-05-11 (post-implementation, mid-PR)

Empirical smoke against both the synthetic 60s en+es concat AND the maintainer's real 3-min bilingual recording proved that the confidence-based trigger alone (D1, algorithm step 7) DOES NOT close the continuous code-switch case. Per-segment `Segment.confidence` (probability-space mean of `tok.token_probability()`) was uniformly above 0.5 on Spanish-audio-transcribed-as-English (min observed: 0.83). The D7-deferred `.plog` upgrade was also tested empirically and found insufficient (min log-prob mean: -0.28, well above any usable threshold). Whisper is genuinely confident at the token level when hallucinating the wrong language.

A **duration-based force-split** was added alongside the confidence trigger to close the gap. New constant `MAX_AUTO_SPLIT_MS = 30_000` (mirroring Whisper's ~30s decoder context). In auto-detect mode (no language hint), any region longer than this is bisected at its midpoint regardless of per-segment confidence. The confidence trigger remains as a safety net for genuinely confused short regions.

Spec deltas:
- **D1** now reads "Hybrid trigger: confidence-based recursion + duration-based force-split for long auto-detect regions."
- **D6** constants list adds `MAX_AUTO_SPLIT_MS = 30_000`.
- **Algorithm step 7** now reads: split if `worst.confidence < THRESHOLD` OR `region.duration > MAX_AUTO_SPLIT_MS && language_hint.is_none()`. Confidence trigger splits at the lowest-confidence segment's start; duration trigger splits at the midpoint (because the worst-segment heuristic can't be trusted when Whisper is uniformly confident in a wrong-language transcription).
- `transcribe_recursive` short-circuits the duration trigger for `is_fake()` adapters so deterministic test fakes (which return canned segments regardless of input) don't get duplicated across split halves.

The rest of the spec stands as written. Risks #1 and #2 were prescient — see PR #115 for the smoke evidence that drove the amendment.

## Problem

Slice 07 shipped VAD-aware per-region ASR with auto-detect: VAD splits audio into speech regions, regions are merged across gaps <5s, each merged region runs Whisper with `language_hint = None`. This works when languages are separated by ≥5s of silence.

It fails on continuous speech mid-utterance code-switching. Real-world evidence: `/Users/fran/Movies/2026-05-04 12-41-37.mov` switches from English to Spanish around the 21:30 mark with no silence gap. VAD merges everything into one large region. Whisper, given the whole region, picks `en` (probability 0.999) for the entire call and either drops the Spanish or translates it to English.

The regression baseline test `continuous_bilingual_detects_only_first_language` synthetically reproduces this: `en_30s.wav` concatenated with `es_30s.wav` (zero gap) → current pipeline detects only `en`. Slice 07 pinned this as a regression baseline and deferred the fix to the present slice.

## Goal

When a merged speech region contains a likely mid-region language switch, split the region at the switch point and re-transcribe each half independently. Each half gets its own language auto-detect call. Stitch the per-half transcripts back together with absolute-time offsets.

## Decisions

| # | Decision | Reason |
|---|---|---|
| D1 | **Confidence-based recursion**, not fixed-window splitting | Adaptive: monolingual audio pays no extra cost; the perf concern raised post-slice-07 wins. Pure midpoint bisection would force splits even when not needed. |
| D2 | **Split point = lowest-confidence segment's `start_ms`** within the region | Aligns the split with where Whisper actually struggled. Likely the language boundary. Cheap to compute from the transcript we already have. |
| D3 | **Signal = `Segment.confidence`** (probability-space mean of `tok.token_probability()`) | Already populated by `WhisperRsAsr`. No adapter API change this slice. The `.plog` upgrade is filed as deferred debt. |
| D4 | **Free function in `panops-portable`**, not a new trait/wrapper | Pure orchestration around `&dyn AsrProvider`. Matches slice-07 D8 (`merge_adjacent_regions`) precedent. No pre-trait. |
| D5 | **Auto-mode only** — recursion is skipped when a language hint is passed | Recursion exists to fix auto-detect failures. With a hint set, Whisper isn't auto-detecting, so the signal is meaningless. Also avoids surprising users who passed `--language en` and would expect single-language output. |
| D6 | **Constants, not flags**: `THRESHOLD = 0.5`, `MIN_REGION_MS = 10_000`, `MAX_DEPTH = 3` | Mirrors slice-07's hardcoded 5_000ms merge gap. Flags add API surface; tuning is a follow-up debt issue if real recordings show false positives or negatives. |
| D7 | **Defer `.plog` / avg_logprob signal upgrade** to a follow-up issue | Bigger semantic change (log-probability scale != [0,1] probability). Different threshold default. Out of slice 08 to keep the diff focused. |

## Scope

### In

1. New module `crates/panops-portable/src/recursive_asr.rs` exporting a single free function `transcribe_recursive`.
2. New `LowConfidenceAsr` fake in `crates/panops-core/src/conformance/fakes.rs` for unit-testing recursion without ML inference.
3. New `crates/panops-portable/tests/recursive_asr.rs` with four unit tests (high-confidence passthrough, single-split case, `MAX_DEPTH` cap, `MIN_REGION_MS` floor).
4. Wire `transcribe_recursive` into both call sites that today call `asr.transcribe` per merged region:
   - `crates/panops-engine/src/server/handlers.rs::run_notes_pipeline` (the per-region loop after `merge_adjacent_regions`).
   - `crates/panops-engine/src/main.rs::transcribe_with_vad` (CLI default + notes mode share this).
5. Flip the regression baseline test `continuous_bilingual_detects_only_first_language` to its positive form (both `en` and `es` detected). Rename to `continuous_bilingual_recursively_detects_both_languages`.
6. Real-audio smoke against two recordings before PR opens:
   - `/Users/fran/Movies/2026-05-04 12-41-37.mov` (bilingual, around the 21:30 EN→ES boundary).
   - `/Users/fran/Movies/2026-05-08 19-04-03.mov` (monolingual English, no regression — no spurious `es` segments).

### Out (filed as debt if surfaced)

- Tuning `THRESHOLD` / `MIN_REGION_MS` / `MAX_DEPTH` against a broader corpus of real recordings.
- Swapping `tok.token_probability()` for `tok.token_data().plog` and re-tuning the threshold. Tracked as a debt issue.
- Per-language Whisper model selection (slice 07 Open Q4).
- Real-time / streaming variant of recursion (Anchor B).
- WhisperKit Mac sidecar equivalent (Anchor A).
- Any IPC method or wire-protocol change. Recursion is internal orchestration only; clients see the same `Transcript` shape.
- Confidence-recursion exposed as a tunable CLI flag.

## Architecture

```
┌─────────────────┐    ┌────────────────────┐    ┌───────────────────────┐
│ VAD detects     │    │ merge_adjacent_    │    │ transcribe_recursive  │
│ raw regions     │───▶│ regions(gap=5s)    │───▶│ (per merged region)   │
└─────────────────┘    └────────────────────┘    └──────────┬────────────┘
                                                            │
                                                            ▼
                                                ┌───────────────────────┐
                                                │ stitch segments       │
                                                │ with absolute offsets │
                                                └───────────────────────┘
```

`transcribe_recursive` is the only new piece. The pipeline shape is unchanged.

### Function signature

```rust
// crates/panops-portable/src/recursive_asr.rs
pub struct RegionResult {
    pub segments: Vec<Segment>,
    pub model: Option<String>,
}

pub fn transcribe_recursive(
    asr: &dyn AsrProvider,
    samples: &[f32],
    sample_rate: u32,
    region: SpeechRegion,
    language_hint: Option<&str>,
    depth: u32,
) -> Result<RegionResult, AsrError>;

const THRESHOLD: f32 = 0.5;
const MIN_REGION_MS: u64 = 10_000;
const MAX_DEPTH: u32 = 3;
```

`RegionResult.segments` has `start_ms` / `end_ms` already absolute (offset by `region.start_ms`). `RegionResult.model` is the model string from the first non-empty `asr.transcribe` call inside the recursion (preserving the slice-07 Copilot round 2 model-provenance fix). Callers concatenate `segments` across regions and pick the first non-empty `model` for the final `Transcript.model`. The tuple-of-vec-and-option shape is intentionally not its own trait — it's a free function's return type only.

### Algorithm

```
fn transcribe_recursive(asr, samples, sr, region, hint, depth):
    1. If hint.is_some():
         delegate to asr.transcribe; offset; return (recursion disabled by D5).
    2. chunk = samples[ms_to_sample(region.start_ms)..ms_to_sample(region.end_ms)]
    3. t = asr.transcribe(chunk, sr, None)?
    4. for seg in t.segments: offset by region.start_ms
    5. If t.segments.is_empty()
          OR depth >= MAX_DEPTH
          OR region.duration_ms < 2 * MIN_REGION_MS:
            return t.segments
    6. worst = t.segments.iter().min_by(|a, b| a.confidence.total_cmp(&b.confidence))
    7. If worst.confidence >= THRESHOLD:
            return t.segments
    8. split_abs_ms = worst.start_ms.clamp(
            region.start_ms + MIN_REGION_MS,
            region.end_ms - MIN_REGION_MS,
       )
    9. left = SpeechRegion { start_ms: region.start_ms, end_ms: split_abs_ms }
       right = SpeechRegion { start_ms: split_abs_ms, end_ms: region.end_ms }
   10. concat(
           transcribe_recursive(asr, samples, sr, left, None, depth + 1)?,
           transcribe_recursive(asr, samples, sr, right, None, depth + 1)?,
       )
```

Step 8's clamp guarantees both halves are ≥ `MIN_REGION_MS` so neither recurses below the floor immediately. Step 5's `region.duration_ms < 2 * MIN_REGION_MS` short-circuits before splitting at all.

### Call-site changes

`run_notes_pipeline` (handlers.rs) and `transcribe_with_vad` (main.rs) replace the inner per-region call:

```rust
// Before (slice 07)
let region_t = asr.transcribe(chunk, sample_rate, params.language.as_deref())?;
if stitched_model.is_none() && !region_t.segments.is_empty() {
    stitched_model = Some(region_t.model.clone());
}
for mut seg in region_t.segments {
    seg.start_ms = (seg.start_ms + region.start_ms).min(total_audio_ms);
    seg.end_ms = (seg.end_ms + region.start_ms).min(total_audio_ms);
    stitched_segments.push(seg);
}

// After (slice 08)
let result = panops_portable::recursive_asr::transcribe_recursive(
    asr,
    &samples,
    sample_rate,
    *region,
    params.language.as_deref(),
    0,
)?;
if stitched_model.is_none() {
    stitched_model = result.model;
}
stitched_segments.extend(result.segments);
```

Note: the absolute-time offset is now applied INSIDE `transcribe_recursive`, so the existing `seg.start_ms + region.start_ms` adjustment is removed at both call sites. Catching this is a deliberate plan step.

## Testing

### Unit tests (cheap, no ML)

`crates/panops-portable/tests/recursive_asr.rs` against `LowConfidenceAsr` fake:

1. **High-confidence passthrough** — fake returns one segment with `confidence = 0.9`. Recursion returns it unchanged. Asserts no extra `asr.transcribe` calls beyond the first.
2. **Single split** — fake returns two segments on the first call: `en` at 0-15s with `confidence = 0.3`, `es` at 15-30s with `confidence = 0.3`. After split, each half returns one high-confidence segment in its own language. Asserts final transcript has both `en` and `es`.
3. **MAX_DEPTH cap** — fake always returns low-confidence segments. Recursion stops at depth 3. Asserts the segments returned are from depth-3 attempts (not infinite recursion).
4. **MIN_REGION_MS floor** — region duration = 15s (less than `2 * MIN_REGION_MS`). Recursion returns the original transcript without splitting even if confidence is low.

### Integration test (existing, flipped)

`crates/panops-engine/tests/ipc_notes_generate_bilingual_per_region_language.rs::continuous_bilingual_detects_only_first_language`:

- Rename to `continuous_bilingual_recursively_detects_both_languages`.
- Flip the assertion from negative (`!langs.contains("es")`) to positive (`langs.contains("en") && langs.contains("es")`).
- Update the docstring to reflect that this is now the green path for the slice-08 capability.

### Existing tests that must stay green

- `bilingual_audio_yields_per_region_language_attribution` (silence-gap case from slice 07). Recursion should not trigger on this — each region is already short and presumably high-confidence. If it triggers and the test flakes, the threshold or min-size is wrong.
- All `vad_conformance` tests.
- All `ipc_notes_generate_*` tests.

### Real-audio smoke (manual, pre-PR)

Two CLI runs from a clean tempdir, results eyeballed:

```bash
./target/release/panops-engine notes --llm-provider ollama \
  "/Users/fran/Movies/2026-05-04 12-41-37.mov"
./target/release/panops-engine notes --llm-provider ollama \
  "/Users/fran/Movies/2026-05-08 19-04-03.mov"
```

Pass criteria:

- The bilingual recording's `transcript.json` shows segments tagged `en` AND `es`; `notes.md` frontmatter `languages:` lists both. Visual inspection of segments around 21:30 shows the language flip near the recording's actual flip.
- The English-only recording's `transcript.json` shows only `en`; `notes.md` frontmatter `languages: [en]`.

Both smokes captured in the session log when the slice ships.

## Three-tier boundaries

### ✅ Always do

- Run `cargo fmt && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` before marking any task done.
- Commit per task in the slice plan.
- Open a GitHub issue for any "deferred" / "out of scope" / "follow-up" item that surfaces during implementation.
- Preserve `Segment.confidence` population in `WhisperRsAsr` exactly as it is today (probability-space mean from `tok.token_probability()`).
- Wire the new function into BOTH the IPC handler and the CLI in the same task. Don't ship recursion in one path and not the other.

### ⚠️ Ask first

- Swapping `tok.token_probability()` for `tok.token_data().plog` to change the confidence signal scale.
- Changing the `THRESHOLD`, `MIN_REGION_MS`, or `MAX_DEPTH` constants once they're committed (real-audio smoke evidence required).
- Adding a CLI / IPC flag to override any of the three constants.
- Renaming a `panops-core` public type (e.g., adding fields to `SpeechRegion` or `Segment`).
- Bundling a non-slice-08 fix into the PR.

### 🚫 Never do

- Introduce a `RecursiveAsr` trait or wrapper. The free function is the answer; pre-trait is the slice-07 anti-pattern restated.
- Make `transcribe_recursive` hold thread-local state, memoize across calls, or cache transcripts.
- Recurse when a language hint is set (D5 enforcement).
- Change `AsrProvider::transcribe`'s signature. Slice 07 just reshaped it; another churn this slice is out.
- Pre-mel-encode or pre-cache audio. Performance is Anchor A's problem.
- Phone home or add telemetry around confidence thresholds (zero-telemetry, ever).
- Open or merge the PR autonomously. The maintainer opens PRs and merges.

## Acceptance criteria

1. `continuous_bilingual_recursively_detects_both_languages` (renamed integration test) passes on CI for macOS.
2. `bilingual_audio_yields_per_region_language_attribution` and all `vad_conformance` / `ipc_notes_generate_*` tests stay green.
3. Four new `recursive_asr` unit tests pass.
4. Manual smoke on the bilingual recording shows `en` AND `es` in transcript and notes frontmatter.
5. Manual smoke on the monolingual recording shows `en` only in transcript and notes frontmatter (no false positives).
6. `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `THRESHOLD = 0.5` is wrong for real audio (too high → too many splits; too low → no splits when needed) | Medium | Pre-PR smoke on two recordings. If the bilingual one doesn't trigger or the monolingual one over-triggers, retune before opening the PR. Filed as `Ask first` boundary. |
| Whisper "translates" Spanish to confident English (mean token probability stays high despite wrong language) | Low–Medium | If smoke shows this, the fix is the `.plog`-signal upgrade (D7 deferred). Threshold tuning won't help that case. Documented as a known limitation if observed during smoke. |
| Recursion stitching produces overlapping or out-of-order timestamps | Low | Step 8's clamp + the `start_ms` total ordering preserved by `min_by` rule out overlap. Unit test #2 verifies. |
| Existing per-region loop in IPC / CLI duplicates segments because `transcribe_recursive` already absolute-times its output | Low | Implementation note in the plan: the wiring step removes the existing `seg.start_ms + region.start_ms` adjustment at both call sites. Caught by `ipc_notes_generate_bilingual_per_region_language` regressing if missed. |
| Smaller Whisper model defaults (if maintainer drops to medium/small for perf) interact badly with confidence threshold | Low | Threshold is calibrated against the current large-v3-turbo q5_0 default. If the model changes, retune (already an `Ask first` boundary). |

## Open questions (deferred to future slices)

1. Should the `confidence` signal switch to `.plog`-based avg_logprob for closer Whisper-internal-semantics alignment? File as `type:debt severity:low area:asr`.
2. Should the recursion expose a CLI / IPC override for the threshold? Probably yes for power users; file as `type:feature severity:low area:asr` once real-meeting evidence accumulates.
3. Is `MAX_DEPTH = 3` enough for real recordings with 3+ language switches in a single VAD region? Likely yes (16 sub-regions max), but worth re-evaluating after smoke.
4. Does per-language model selection (slice 07 Open Q4) interact with recursion? Probably orthogonal but flag during slice 09+ brainstorm.
