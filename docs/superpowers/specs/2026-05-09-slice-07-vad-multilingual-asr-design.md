# Slice 07 — VAD-aware multilingual ASR: design

**Status:** locked. Amendments require a maintainer decision recorded inline with a date stamp.
**Closes:** the multilingual-day-1 north-star gap surfaced by the maintainer's `2026-05-08 19-04-03.mov` real-meeting test on 2026-05-09 (Whisper detects ONE language for the whole file and transliterates the other half).
**Brainstorm:** `docs/superpowers/specs/2026-05-09-slice-07-vad-multilingual-asr-brainstorm.md`.
**Slice tracking issue:** TBD (open with `gh issue create --label type:feature` post-spec-approval).

## Why this shape

Six load-bearing decisions, each with an alternative considered and rejected.

1. **VAD via `whisper-rs::WhisperVadContext`.** Same crate as the ASR adapter (`whisper-rs 0.16`), single C++ call path, no marshaling between two C++ libraries. Rejected: `sherpa-rs::silero_vad` (extra wrapper + sample-buffer marshaling); standalone `silero-vad-rs` (new top-level dep).

2. **Extract `Vad` as a first-class port in `panops-core`.** This is a **deliberate departure from AGENTS.md "NEVER pre-trait for hypothetical future adapters"** — but Anchor B (live capture) is committed via the north-star (acceptance #1 + the explicit Anchor B in `AGENTS.md` trajectory), not hypothetical. Slice 07 and Anchor B share the same `Vad` abstraction, avoiding a refactor when Anchor B lands. Recorded as a ratified maintainer decision (2026-05-09).

3. **Samples-based `AsrProvider` trait + external pipeline orchestration.** Reshape the ASR port from `transcribe_full(audio_path, language_hint)` to `transcribe(samples, sample_rate, language_hint)`. Pipeline (in `crates/panops-engine`) composes `vad → for region → asr`. Single-responsibility per port; Anchor B (live capture) feeds samples directly with no file dance. Rejected: VAD internal to `WhisperRsAsr` (port hidden inside one consumer); hybrid file + samples API (trait widens; two paths to maintain).

4. **5s region-merge gap (whisperX default).** Whisper needs ~30s of speech for reliable language detection. Adjacent VAD regions with gaps <5s merge into one before passing to Whisper, so detection lands on enough speech to be reliable. Rejected: 2s (regions too short for reliable detect); 10s (mid-pause language switches lost in merged regions).

5. **`--language X` forces ALL regions.** Monolingual escape hatch + explicit override of Whisper's guess. Absence of the flag triggers per-region auto-detect (the new bilingual default). Rejected: fallback-hint semantics (more complex contract); removal (loses escape hatch).

6. **No confidence-based recursion in this slice.** Baseline (VAD → per-region detect+transcribe → stitch) lands the multilingual-day-1 north-star promise at 30s granularity. If real bilingual meetings show 30s isn't fine enough, recursion is its own follow-up slice. Rejected: bundling recursion (more decision points to validate before knowing baseline alone is enough).

## What the maintainer actually said

Per the May 2 alignment audit's recommendation #2 ("add a 'what the user actually said' section to each slice spec"), here are the maintainer's verbatim decisions captured during the 2026-05-09 brainstorm:

> **Question:** Which VAD source for slice 07's speech-region detection?
> **Maintainer:** "whisper-rs's WhisperVadContext (Recommended)"

> **Question:** How wide should slice 07's scope be?
> **Maintainer:** "Bigger refactor: extract Vad port"

> **Question:** Where does the VAD orchestration live, and what shape does the AsrProvider trait take?
> **Maintainer:** "External orchestration, samples-based ASR (Recommended)"

> **Question:** Region merge threshold?
> **Maintainer:** "5s (whisperX default, Recommended)"

> **Question:** What does the existing `--language` CLI flag mean once VAD-aware auto-detect is in place?
> **Maintainer:** "Forces all regions to that language (Recommended)"

The maintainer also explicitly acknowledged that extracting the `Vad` port now (decision D2) departs from AGENTS.md "NEVER pre-trait" and ratified the deviation on the basis of Anchor B's commitment in the north-star.

Everything else (concrete trait shapes, error variants, the `merge_adjacent_regions` helper, audio-loading utility extraction location, `EngineServices.heavy` placement, `TranscriptFileFake` rewrite shape) is an assistant default the maintainer accepted by approving this spec.

## Scope (in this slice)

- New port `Vad` in `panops-core::vad` with associated types (`SpeechRegion`, `VadError`).
- New conformance harness `panops-core::conformance::vad::run_suite`.
- New fake `KnownRegionsFake` in `panops-core::conformance::fakes` (returns canned regions; satisfies the harness; lets engine integration tests swap real VAD without loading a model).
- New real impl `WhisperVad` in `panops-portable` wrapping `WhisperVadContext`.
- VAD model download via `panops-portable::model` machinery (mirror of how `ggml-large-v3-turbo` is fetched).
- **`AsrProvider` trait reshape** from `transcribe_full(audio_path, language_hint)` to `transcribe(samples: &[f32], sample_rate: u32, language_hint: Option<&str>)`. Cascades through:
  - `WhisperRsAsr` rewritten to take samples; `language_hint=None` invokes per-call language detection (`set_language(None)` + `set_detect_language(true)`); `language_hint=Some(X)` forces.
  - `TranscriptFileFake` rewritten for the samples-based contract.
  - `panops-core` ASR conformance harness updated.
  - All existing call sites (CLI `transcribe()`, IPC `run_notes_pipeline`) updated.
- New audio-loading utility (extracted from current `WhisperRsAsr::transcribe_full`'s file-open path) that returns `(Vec<f32>, u32)`. Lives in `panops-portable` (e.g., `panops_portable::audio::load_wav_mono16k`).
- Pipeline rewire: load samples → `vad.detect_speech` → `merge_adjacent_regions(regions, 5000)` → for each merged region, `asr.transcribe(slice, sr, language_hint)` → stitch transcripts with absolute-time offsets → continue with diar / notes pipeline.
- `Segment.language_detected` finally varies per segment honestly (was a copy of file-level guess).
- `EngineServices.heavy` gains `vad: Arc<dyn Vad>` field.
- New `From<VadError> for IpcError` in `panops-protocol::error` behind the `domain-conversions` feature flag.
- CLI `--language` flag stays; semantics formalized as override-all-regions when set.
- Docs: `docs/proto/ipc.md` updated only if any IPC error / event shape changes (none planned for this slice; the wire shape is unchanged).

## Out of scope (defer; file as `type:debt` issues at slice end)

- **Confidence-based region splitting / recursion.** Real-meeting evidence first. If 30s granularity isn't fine enough on the maintainer's bilingual meetings, that's a follow-up slice.
- **Anchor B (live capture).** `Vad` port is shaped for it; the consumer wiring isn't.
- **Per-language Whisper model selection.** All transcription uses the single bundled multilingual model.
- **Diarization rework** (already filed as #107 against pyannote 4.0 / community-1).
- **LLM model evaluation** (already filed as #108).
- **ASR backend alternatives** for English-only / real-time paths (already filed as #109).
- **Whisper bundled VAD model fallback to a different VAD provider.** YAGNI.
- **Audio resampling / channel mixing.** The CLI already requires 16 kHz mono WAV; `WhisperVad` and `WhisperRsAsr` both assume that. If a future input doesn't match, surface a clear `InvalidAudio` error. Anchor B's live-capture path will produce 16 kHz mono samples directly.

## Architecture

### Pipeline (orchestration in `panops-engine`)

```
┌─ pipeline (handlers.rs / main.rs) ────────────────────────────────┐
│ load_audio(path) -> (samples: Vec<f32>, sample_rate: u32)         │
│         │                                                         │
│         ▼                                                         │
│ vad.detect_speech(&samples, sample_rate) -> [SpeechRegion]        │
│         │                                                         │
│         ▼                                                         │
│ merge_adjacent_regions(regions, gap_ms=5000) -> [SpeechRegion]    │
│         │                                                         │
│         ▼                                                         │
│ for region in merged_regions:                                     │
│     let chunk = &samples[ms_to_samples(region.start_ms)            │
│                          ..ms_to_samples(region.end_ms)];         │
│     let t = asr.transcribe(chunk, sample_rate, language_hint);    │
│     // t.segments offsets are 0-based within the region          │
│     for seg in t.segments:                                        │
│         seg.start_ms += region.start_ms;                          │
│         seg.end_ms   += region.start_ms;                          │
│         stitched.push(seg);                                       │
│         │                                                         │
│         ▼                                                         │
│ Transcript { segments with per-segment language_detected }        │
│         │                                                         │
│         ▼                                                         │
│ (existing) diarize → merge_speaker_turns → NotesGenerator …       │
└───────────────────────────────────────────────────────────────────┘
```

`load_audio` and `merge_adjacent_regions` are small free functions in `panops-portable::audio`. They can be unit-tested without any model.

### Ports

```rust
// panops-core/src/vad.rs (new)
pub trait Vad: Send + Sync {
    fn detect_speech(
        &self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<SpeechRegion>, VadError>;

    fn is_fake(&self) -> bool { false }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechRegion {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum VadError {
    #[error("vad model: {0}")]
    Model(String),
    #[error("invalid audio: {0}")]
    InvalidAudio(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
// MUST NOT derive Serialize per AGENTS.md.

**Amended 2026-05-09 (post-implementation):** `VadError::Io` is a tuple variant matching `AsrError::Io` and `DiarError::Io` conventions, not the struct variant shown earlier in design drafts. Adopting the convention keeps the future `From<VadError> for IpcError` mapping consistent with sibling domain-error mappings.
```

```rust
// panops-core/src/asr.rs (CHANGED — was file-based)
pub trait AsrProvider: Send + Sync {
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError>;

    fn is_fake(&self) -> bool { false }
}
```

`Transcript` shape unchanged. `Segment.language_detected` finally populated per-segment (Whisper's per-call language ID).

### `From<VadError> for IpcError`

In `panops-protocol::error::from_domain` (gated by `domain-conversions`):

| `VadError` | `IpcError` | Wire message |
|---|---|---|
| `Model(_)` | `Internal` | `"vad model error"` |
| `InvalidAudio(m)` | `InvalidInput` | `m` (already user-safe; mirrors `AsrError::InvalidAudio`) |
| `Io(_)` | `Internal` | `"vad io error"` |

Per the slice-05 hardening pattern: full detail → `tracing::error!`; wire message stays opaque.

### `merge_adjacent_regions`

Pure function; lives in `panops-portable::audio` next to `load_audio`. Pseudocode:

```rust
pub fn merge_adjacent_regions(
    mut regions: Vec<SpeechRegion>,
    gap_ms: u64,
) -> Vec<SpeechRegion> {
    regions.sort_by_key(|r| r.start_ms);
    let mut out: Vec<SpeechRegion> = Vec::new();
    for r in regions {
        if let Some(last) = out.last_mut() {
            if r.start_ms.saturating_sub(last.end_ms) <= gap_ms {
                last.end_ms = last.end_ms.max(r.end_ms);
                continue;
            }
        }
        out.push(r);
    }
    out
}
```

Const for the threshold lives in `panops-engine`'s pipeline site (so a future config could surface it, but no env var per AGENTS.md).

### `EngineServices.heavy` extension

`HeavyAdapters` (in `panops-engine/src/server/mod.rs`) gains a `vad: Arc<dyn Vad>` field. Constructed alongside `asr` / `diar` / `exporter` in `init_heavy_adapters`. The VAD model load is comparable to Sherpa's (~hundreds of ms), so it lives in the heavy-init path that runs concurrent with the accept loop.

### Audio loading utility

```rust
// panops-portable/src/audio.rs (new)
pub fn load_wav_mono16k(path: &Path) -> Result<(Vec<f32>, u32), AudioError> {
    // wraps hound::WavReader, validates 16 kHz mono PCM, returns f32 samples
}
```

Used by both CLI default mode (`run_default`), CLI notes mode (`run_notes`), and IPC `run_notes_pipeline`. The current `WhisperRsAsr::transcribe_full`'s internal file-open + sample-decoding logic is the source.

### CLI behavior

| Surface | Before slice 07 | After slice 07 |
|---|---|---|
| `panops <wav>` (default) | Whisper detects ONE language for the whole file | Pipeline does `vad → per-region detect+transcribe → stitch`. Without `--language`, segments carry per-region detected language. With `--language X`, all regions transcribe in X. |
| `panops notes <wav>` | Same as above + diar + LLM notes | Same upgrade applied. The notes pipeline downstream is unchanged (it consumes a `Transcript`). |
| `panops --language <lang>` | Was a fallback hint; behavior depended on Whisper's first-30s detect | Now an explicit override: forces every region to that language. |
| `panops-engine serve` (IPC) | `notes.generate` ran the same one-language pipeline | Same VAD-aware upgrade. Wire shape unchanged. |

## Test surface

PR-gating tests:

**Unit (panops-core):**
1. `vad_conformance::run_suite` — runs against `KnownRegionsFake` (and `WhisperVad` via the panops-portable adapter test crate). Asserts: returned regions are sorted, non-empty samples produce at least one region, all `start_ms < end_ms`, regions don't overlap.
2. `Segment` shape unit test — `language_detected: Option<String>` round-trips; varies between segments in a multi-region transcript.

**Unit (panops-portable):**
3. `merge_adjacent_regions`: input `[(0..2000), (3000..6000), (15000..20000)]` with gap 5s → output `[(0..6000), (15000..20000)]` (first two merged because 1s gap < 5s; last stays separate because 9s gap > 5s).
4. `merge_adjacent_regions` empty input → empty output.
5. `merge_adjacent_regions` single region → unchanged.
6. `load_wav_mono16k` unit tests: rejects non-16kHz, rejects stereo, rejects non-WAV, accepts the existing `tests/fixtures/audio/en_30s.wav`.

**Integration (panops-portable):**
7. `whisper_rs_asr_passes_conformance` — adapter satisfies the new samples-based AsrProvider conformance suite.
8. `whisper_vad_passes_conformance` — adapter satisfies the Vad conformance suite (loads the bundled VAD model).
9. `whisper_rs_asr_detects_language_per_call` — pass `language_hint=None` with EN-only samples → result has `language_detected: Some("en")`. Pass with ES-only samples → `Some("es")`.

**Integration (panops-engine):**
10. **Bilingual smoke test**: synthetic WAV concatenating `tests/fixtures/audio/en_30s.wav` + `tests/fixtures/audio/es_30s.wav`. Run the full pipeline (`vad → per-region transcribe → stitch`); assert that segments before the boundary have `language_detected: Some("en")` and segments after have `language_detected: Some("es")`. Tolerate ±2 segments of misattribution at the boundary (Whisper's per-30s detect isn't pixel-perfect).
11. Existing slice-04 fixtures (`en_30s.wav`, `multi_speaker_60s.wav`) MUST still produce correct single-language output (no regressions).
12. `--language en` flag forces every region to `en` (override path) — verified against the bilingual synthetic WAV: with `--language en`, the ES half should still appear as `language_detected: Some("en")` (Whisper's transliteration of Spanish under EN forcing).

**Manual smoke** (against the maintainer's `2026-05-08 19-04-03-full.wav`):
13. After implementation, re-run the full notes pipeline. Assert via the `Transcript` JSON that `language_detected` actually has BOTH `"en"` AND `"es"` values across the segments.

## Implementation order (sketch)

Canonical task list goes in the writing-plans output. This is illustrative.

1. Add `Vad` port + `SpeechRegion` + `VadError` to `panops-core`. Compile + clippy clean. No impl yet.
2. Add `vad_conformance::run_suite` harness (compile only, no caller).
3. Add `KnownRegionsFake` to `panops-core::conformance::fakes`. Wire into harness; cargo test passes.
4. Add `audio::load_wav_mono16k` + `audio::merge_adjacent_regions` utilities to `panops-portable`. Unit tests pass.
5. Add `WhisperVad` real adapter to `panops-portable`. Add VAD model download to `panops-portable::model`. Conformance test against real adapter passes.
6. Reshape `AsrProvider` trait to samples-based. Update `panops-core::conformance::asr` accordingly. (Will break compilation; tasks 7-10 fix it.)
7. Rewrite `WhisperRsAsr.transcribe` to take samples + per-call language detection. Internal file-loading code moves to `audio::load_wav_mono16k` (already done in step 4).
8. Rewrite `TranscriptFileFake` for samples-based contract. The `<audio>.transcript.txt` sidecar mechanism becomes a small test helper that constructs the canned `Transcript` (not an `AsrProvider` impl).
9. Add `From<VadError> for IpcError` mapping + tests.
10. Extend `EngineServices.heavy` with `vad: Arc<dyn Vad>`. Construct in `init_heavy_adapters`.
11. Rewrite `run_notes_pipeline` (handlers.rs): `load_audio → vad → merge → for-region transcribe → stitch → diar → notes`.
12. Rewrite CLI `transcribe()` (main.rs) similarly. Update the `PANOPS_FAKE_ASR=1` path to use the new fake.
13. Add the bilingual synthetic-WAV integration test.
14. Update existing slice-04 / slice-05 / slice-06 IPC integration tests to the new `EngineServices` shape.
15. Run manual smoke against the maintainer's bilingual recording; capture output for the slice-boundary audit.

## Three-tier boundaries

Per AGENTS.md "every slice spec MUST define them".

### ✅ Always do (no per-decision approval needed)

- Run `cargo fmt && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` before claiming any task done.
- Use `tempfile::TempDir` for every test that touches disk.
- Sanitize wire-side error messages: opaque "vad/asr error" externally; full detail to `tracing::error!`.
- Commit per task in the slice plan.
- File a follow-up `type:debt` GitHub issue for any "deferred" / "out of scope" item discovered during implementation.
- Run `vad_conformance` against both `KnownRegionsFake` and `WhisperVad`.
- Run the (now samples-based) ASR conformance against `TranscriptFileFake` and `WhisperRsAsr`.
- Drive `OnceLock` / `OnceCell` slots to a terminal state on every path including panic (per AGENTS.md `OnceLock` rule).

### ⚠️ Ask first

- Renaming a public protocol type or domain-error variant.
- Introducing a new top-level dep beyond `whisper-rs`'s built-in VAD support (e.g., switching to `silero-vad-rs` or adding a separate VAD crate).
- Choosing an alternative VAD model (the bundled whisper.cpp Silero VAD model is the chosen baseline).
- Bundling a non-slice-07 issue into the slice (confidence recursion, NotionEnhanced default, diar upgrade, LLM evaluation, etc.).
- Changing the region-merge gap from 5s after the first commit lands.
- Touching the LLM stage (out of slice scope).
- Modifying the `Transcript` or `Segment` shape (downstream pipeline depends on stability).

### 🚫 Never do

- Add an env var for VAD config, region-merge gap, or any user-facing behavior (per drift §1, AGENTS.md, north-star).
- Pre-trait additional VAD variants beyond what `Vad` already covers (the port shape was chosen to be reusable; adding sub-traits is YAGNI).
- Add confidence-based recursion in this slice (deferred to a follow-up slice).
- Auto-merge a PR (slice-05 lesson).
- Auto-file new architectural concerns as issues without surfacing to the maintainer first (slice-05 audit §5).
- Phone home or log to disk anything that contains user content beyond what's already persisted.
- Derive `serde::Serialize` on `VadError` or any other domain error (per AGENTS.md).
- Open a PR autonomously. The maintainer opens PRs.

## Decisions (locked)

- **D1**: VAD source = `whisper-rs::WhisperVadContext` (whisper.cpp's bundled Silero VAD). Reason: zero new top-level deps, single C++ call path.
- **D2**: Extract `Vad` as a first-class port in `panops-core`. **Deliberate departure from "NEVER pre-trait"** — Anchor B (live capture) is committed via the north-star, not hypothetical. Reason: future-proofs the architecture; slice 07 + Anchor B share the same abstraction.
- **D3**: Reshape `AsrProvider` to samples-based (`transcribe(samples, sample_rate, language_hint)`); pipeline orchestrates `vad → for region → asr` externally. Reason: single-responsibility per port; Anchor B feeds samples directly.
- **D4**: Region-merge gap = 5s (whisperX default). Reason: matches the proven default, gives Whisper enough speech for reliable language detect without crossing long pauses.
- **D5**: `--language X` forces all regions to X. Default (no flag) = per-region auto-detect. Reason: explicit override + monolingual escape hatch.
- **D6**: No confidence-based recursion in this slice. Reason: real-meeting evidence first; baseline lands the north-star promise; recursion is a follow-up slice if needed.
- **D7**: VAD model + `WhisperVad` adapter live in `panops-portable` (mirrors `WhisperRsAsr` placement). Reason: same C++ binding crate; same model-download machinery.
- **D8**: `merge_adjacent_regions` is a pure free function in `panops-portable::audio`, not a method on `Vad`. Reason: deterministic, no model state, composable.
- **D9**: `From<VadError> for IpcError` mapping: `Model` → `Internal` opaque; `InvalidAudio` → `InvalidInput` with message; `Io` → `Internal` opaque. Reason: matches slice-05 / slice-06 transport boundary patterns.
- **D10**: `is_partial` field on `Segment` stays `false` for all batch transcription output. Reason: live capture (Anchor B) will use it; batch transcription has no concept of partial.

## Open questions (out of slice 07; surface for separate decision)

1. **Confidence-based region splitting / recursion.** If real bilingual meetings show 30s VAD granularity is too coarse, this is the follow-up slice. Will know after slice 07 ships and the maintainer re-runs on real audio.
2. **NotionEnhanced as current default** (drift §3 from May 2 audit, still open). Orthogonal to slice 07.
3. **CI integration test against `gemma3:4b` on Ollama** to catch silent LLM regressions. Out of slice scope; valuable but separate.
4. **Per-language Whisper model selection.** All transcription uses the single bundled multilingual model. Future: smaller English-only model on EN regions?

## Done when

- All PR-gating tests pass (vad conformance × 2 adapters + asr conformance × 2 adapters + 4 unit tests on merge / load_audio + bilingual integration test + 2 retained-fixture regression tests + 1 `--language` override test).
- `cargo fmt && cargo clippy --workspace --all-targets --locked -- -D warnings` is clean.
- Manual smoke against the maintainer's `2026-05-08 19-04-03-full.wav`: `Transcript.segments[].language_detected` carries BOTH `"en"` AND `"es"` across the segments.
- Existing slice-06 `panops-engine/tests/ipc_*` tests continue to pass (after their `EngineServices::ready` calls are updated to include the new VAD adapter — small mechanical change).
- Slice-tracking issue closed; project board entry moved to Done.
- Plan file moved to `docs/superpowers/plans/done/07-vad-multilingual-asr.md`.
- Slice-boundary alignment audit run after PR merge, written to `docs/superpowers/reviews/YYYY-MM-DD-slice-07-audit.md`.

## References

- Brainstorm: `docs/superpowers/specs/2026-05-09-slice-07-vad-multilingual-asr-brainstorm.md`
- Slice 06 spec (precedent for spec format + three-tier boundaries): `docs/superpowers/specs/2026-05-05-slice-06-storage-design.md`
- SOTA debt issues filed during slice 06: #107 (diar upgrade), #108 (LLM evaluation), #109 (ASR backend alternatives)
- whisperX (the proven pattern this slice mirrors): https://github.com/m-bain/whisperX
- whisper.cpp built-in VAD: https://github.com/ggml-org/whisper.cpp/issues/3003
- whisper-rs `WhisperVadContext` docs: https://docs.rs/whisper-rs/0.16.0/
- AGENTS.md: workflow contract (especially the `OnceLock` rule, no-Serialize-on-domain-errors rule, no-env-vars rule, and "NEVER pre-trait" rule which D2 deliberately departs from)
- North star: `docs/north-star.md` — multilingual day 1 constraint and v0.1 acceptance #5 (real-meeting validation)
