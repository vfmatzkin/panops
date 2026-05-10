# Slice 07 — VAD-aware multilingual ASR: brainstorm

**Status:** brainstorm artifact. Source for the locked spec at `2026-05-09-slice-07-vad-multilingual-asr-design.md`. Captures the 2026-05-09 interactive brainstorm session that walked the maintainer through VAD source choice, port extraction, pipeline shape, and merge-threshold defaults.

## 0. Context (why this slice exists)

Slice 02 (headless CLI / ASR) shipped Whisper-large-v3-turbo via `whisper-rs` with a one-language-per-file detection model — Whisper auto-detects from the first 30 seconds and forces that language on every segment. The maintainer's north-star is **multilingual day 1, EN/ES bilingual meetings**. Real-meeting test on the maintainer's `2026-05-08 19-04-03.mov` recording (verified end-to-end during slice-06 dogfooding on 2026-05-09) confirmed the bug: a bilingual recording is transcribed as English-only, with Spanish passages transliterated into garbled English-sounding text.

SOTA audit (filed as #107, #108, #109 during slice-06 work) confirmed:
- Whisper-large-v3 / large-v3-turbo is still the right multilingual ASR model in 2026 — the bug is in usage, not the model.
- The dominant production pattern (whisperX, faster-whisper) is **Voice Activity Detection (VAD) → speech regions → per-region language detection → per-region transcription → stitch**. No fixed window — boundaries fall on natural silences.
- Silero VAD is the de-facto VAD for the Whisper ecosystem; available without new top-level deps via `whisper-rs::WhisperVadContext` (whisper.cpp's bundled VAD) or `sherpa-rs::silero_vad` (the diar crate already in our deps).

## 1. VAD source — which crate?

| Option | Capability | Risk | Cleanliness |
|---|---|---|---|
| **A — `whisper-rs::WhisperVadContext`** (chosen) | whisper.cpp's bundled Silero VAD; `WhisperVadSegments` returns regions ready to feed into per-region `whisper.full()` calls | Low. Same crate as ASR, single C++ call path, no marshaling. VAD impl locked to whisper.cpp's bundled version. | Cleanest single-adapter path. |
| B — `sherpa-rs::silero_vad` | Same Silero VAD model whisperX uses; reusable by Anchor B (live capture) without separate dep | Marshaling samples between sherpa (C++) and whisper-rs (C++); two model files to manage | Adds wrapper around sherpa's silero_vad. |
| C — Standalone `silero-vad-rs` | Purpose-built crate; cleanest API surface | New top-level dep + new maintainer surface | Triggers AGENTS.md "Ask first" boundary on dep additions. |

**Maintainer chose A.**

## 2. Slice scope — port extraction or internal-to-adapter?

| Option | Capability | Risk | Cleanliness |
|---|---|---|---|
| Baseline only (internal to `WhisperRsAsr`) | VAD becomes a private detail of the ASR adapter; AsrProvider trait shape unchanged | Lowest | Minimal change |
| Baseline + confidence-based recursion | Tighter intra-utterance code-switching coverage | More decision points (confidence threshold, max recursion depth, alternative-language fan-out); larger validation surface | More moving parts before knowing baseline alone is enough |
| **Bigger refactor: extract `Vad` port** (chosen) | First-class port; reusable by Anchor B (live capture), which the north-star commits to | Deliberate departure from AGENTS.md "NEVER pre-trait for hypothetical future adapters" — but Anchor B is **not** hypothetical, it's a v0.1 anchor | Future-proofs the architecture; slice 07 + Anchor B share the same abstraction |

**Maintainer chose extract-port** (option C). Recorded as a deliberate departure from "NEVER pre-trait" — Anchor B's commitment in the north-star justifies it.

## 3. Pipeline shape — internal orchestration or external?

| Option | Capability | Risk | Cleanliness |
|---|---|---|---|
| **External orchestration, samples-based ASR** (chosen) | AsrProvider becomes `transcribe(samples, sample_rate, language_hint)`; pipeline composes `vad → for region → asr` | AsrProvider trait shape change cascades through fakes + tests | Single-responsibility per port; Anchor B feeds samples directly with no file step |
| Internal to `WhisperRsAsr`, file-based ASR unchanged | Adapter loads file + calls VAD + per-region whisper.full() internally | VAD port hidden inside one consumer; Anchor B has to wire VAD separately | Smaller refactor; less consistent |
| Hybrid (file + samples APIs) | Both methods coexist | Trait widens; default-impl needs both kept in sync | Some duplicated cost |

**Maintainer chose external orchestration** (option A). Pipeline shape (from preview):

```
load_audio(path) -> samples
vad.detect_speech(samples) -> [region_a, region_b, ...]
for region in regions:
  asr.transcribe(slice(samples, region), 16000, None)
    -> Transcript with detected_language per segment
stitch -> Transcript
```

## 4. Region-merge threshold

Whisper needs ~30s of speech for reliable language detection. Adjacent VAD regions with small gaps should merge to avoid feeding Whisper short sub-30s chunks where detection flips between languages randomly.

| Option | Capability | Trade-off |
|---|---|---|
| **5s (whisperX default)** (chosen) | Speech is essentially continuous within 5s; longer pauses likely indicate turn / topic / language change | Proven default; matches whisperX's published behavior |
| 2s (tighter) | Preserves more natural turn boundaries | Risks short regions where lang detect is unreliable |
| 10s (looser) | More speech per region; better lang detect accuracy | A mid-pause language switch ("…okay. Entonces vamos a…") would be missed |

**Maintainer chose 5s.**

## 5. `--language` flag semantics after auto-detect

| Option | Capability | Trade-off |
|---|---|---|
| **Forces all regions to that language** (chosen) | Monolingual escape hatch + explicit override of Whisper's guess | Default (no flag) = per-region auto-detect (the new bilingual default) |
| Treated as fallback hint only | Auto-detect always; flag kicks in only when confidence low | More complex contract |
| Removed / deprecated | Auto-detect always; no override | Loses escape hatch for users who know their audio is monolingual |

**Maintainer chose forces-all** (option A).

## 6. Sub-decisions taken with assistant defaults (subject to maintainer veto via spec review)

- `Vad` trait is sync; async wrapping happens at the handler via `spawn_blocking` (matches existing port shape).
- `SpeechRegion { start_ms, end_ms }` returned by VAD; `merge_adjacent_regions(regions, gap_ms=5000)` is a separate small fn.
- `panops_core::SpeechRegion` defined alongside `Vad` trait.
- `VadError` variants: `Model(String)`, `InvalidAudio(String)`, `Io { source: io::Error }`. NEVER derives `Serialize` (per AGENTS.md domain-error rule).
- New `From<VadError> for IpcError` in `panops-protocol` behind `domain-conversions`.
- Audio-loading utility extracted from `WhisperRsAsr::transcribe_full`'s current file-open path → returns `(Vec<f32>, u32)`. Lives in `panops-portable`.
- Pipeline orchestration belongs in `crates/panops-engine/src/server/handlers.rs::run_notes_pipeline` and CLI `run_default` / `run_notes` (not in `panops-core` — `panops-core` stays platform-free).
- `EngineServices.heavy` gains a `vad: Arc<dyn Vad>` field (heavy because the VAD model loads at startup).
- Existing `TranscriptFileFake` rewritten for samples-based contract; the `<audio>.transcript.txt` sidecar mechanism becomes a small test helper (not an `AsrProvider` impl).
- `is_partial` field on `Segment` stays `false` for batch transcription (already documented as forward-compat for live capture).

## 7. Open questions surfaced (NOT for this slice — surfaced for separate decision)

1. **Confidence-based recursion timing.** Real-meeting evidence will tell us if 30s VAD granularity is too coarse for the maintainer's bilingual meetings. If yes, that's slice 08.
2. **NotionEnhanced as current default** (drift §3 from May 2 audit, still open). Orthogonal to slice 07.
3. **CI integration test against `gemma3:4b` on Ollama** to catch silent LLM regressions. Out of slice scope.
4. **Per-language Whisper model selection.** All transcription uses the single bundled multilingual model in this slice.

## 8. What this artifact is NOT

- Not the locked spec — that's `2026-05-09-slice-07-vad-multilingual-asr-design.md`.
- Not the executable plan — that's `docs/superpowers/plans/07-vad-multilingual-asr.md` (written by `superpowers:writing-plans` post-spec-approval).
- Not authoritative on architecture — the locked spec is.

## 9. References

- Locked spec: `docs/superpowers/specs/2026-05-09-slice-07-vad-multilingual-asr-design.md`
- SOTA audit issues filed during slice 06: #107 (diar upgrade to pyannote 4.0), #108 (LLM evaluation), #109 (ASR backend alternatives)
- Slice 06 spec (precedent for spec format + three-tier boundaries): `docs/superpowers/specs/2026-05-05-slice-06-storage-design.md`
- Real-meeting test artifact (the recording that surfaced the bug): `~/Movies/2026-05-08 19-04-03.mov` (local; not committed)
- whisperX's pattern: https://github.com/m-bain/whisperX
- whisper.cpp built-in VAD: https://github.com/ggml-org/whisper.cpp/issues/3003
- whisper-rs `WhisperVadContext` docs: https://docs.rs/whisper-rs/0.16.0/
