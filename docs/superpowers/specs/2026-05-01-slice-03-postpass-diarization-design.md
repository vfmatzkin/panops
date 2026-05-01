---
project: panops
date: 2026-05-01
status: design (pre-implementation)
slice: 03
authors: Fran Matzkin
type: design
---

# Slice 03 — Post-pass + diarization

## Summary

Adds the `Diarizer` port and a portable adapter (sherpa-onnx via `sherpa-rs`), upgrades the production Whisper model from `tiny.q5_1` to `large-v3-turbo-q5_0`, and merges speaker labels into segments. Slice 03 ships the post-pass-after-recording flow; the rolling-during-recording variant lands in slice 06+ on top of these same ports.

CI gets a model split: production users download `large-v3-turbo-q5_0` (~547 MB) on first run; CI uses `base.q5_1` (~57 MB) routed via `PANOPS_MODEL`. Closes issue #9 along the way.

## Goals

- Land the `Diarizer` trait in `panops-core` (born when needed, not pre-traited).
- Land the portable diarization adapter via `sherpa-rs`.
- Add `speaker_id: Option<u32>` to `Segment`. Engine merges diarizer output into ASR output by timestamp overlap.
- Bump the default Whisper model to `large-v3-turbo-q5_0` for production. CI continues with a smaller model for tractable runtime.
- Add a multi-speaker test fixture and the conformance suite for `Diarizer`.
- Engine binary: `panops-engine <wav>` diarizes by default; `--no-diarize` opts out.
- Keep ports stateless and chunk-friendly so slice 06's live capture can wire rolling-during-recording or at-end without changes here.

## Non-goals

- Live ASR / streaming. No `transcribe_window` work — slice 06/07.
- Rolling re-pass during recording. The orchestrator that schedules post-pass on completed buffers is slice 06+.
- LLM grammar cleanup / notes generation. Slice 04.
- Mac-native diarization (FluidAudio's CoreML diarizer). Slice 06's `panops-mac` adapter.
- Model picker UX in the desktop app. Deferred to slice 06.
- WhisperKit / Argmax SDK integration on Mac. Slice 06.
- ARI/DER metrics with tight thresholds. Slice 03 uses loose structural assertions; tightening lands in a follow-up.

## Architecture (delta from slice 02)

```
┌──────────────────────────────────────┐
│       panops-engine (binary)         │
│  CLI: parse, dispatch ASR + diarize, │
│  merge by timestamp, emit JSON.      │
└──────┬───────────────────┬───────────┘
       │                   │
       ▼                   ▼
┌──────────────┐    ┌──────────────────┐
│ panops-core  │    │ panops-portable  │
│ Segment      │◀───│ WhisperRsAsr     │
│   + speaker  │    │ (impl AsrProv)   │
│ AsrProvider  │    │                  │
│ Diarizer     │◀───│ SherpaDiarizer   │
│ SpeakerTurn  │    │ (impl Diarizer)  │
│ conformance::│    │ model::registry  │
│   asr,diar   │    │   tiny | base |  │
└──────────────┘    │   large-v3-turbo │
                    └──────────────────┘
```

The engine pipeline becomes:

```
audio.wav
  │
  ├──▶ AsrProvider::transcribe_full ─▶ Vec<Segment>  (no speaker_id)
  │
  └──▶ Diarizer::diarize ─────────────▶ Vec<SpeakerTurn>
                                            │
   merge_by_overlap(segments, turns) ───────┘
                                            │
                                            ▼
                                     Vec<Segment> (speaker_id populated)
                                            │
                                            ▼
                                       Transcript JSON
```

## Public contracts

### `Diarizer` trait (`panops-core`)

```rust
pub trait Diarizer {
    /// Run speaker diarization on a complete audio file. Stateless.
    /// Returns a list of speaker-labeled time spans, ordered by `start_ms`,
    /// non-overlapping. Speaker IDs are integers stable within one call;
    /// they have no meaning across calls.
    fn diarize(&self, audio_path: &Path) -> Result<Vec<SpeakerTurn>, DiarError>;

    /// Marker for the conformance harness; production impls leave the default.
    fn is_fake(&self) -> bool {
        false
    }
}
```

### `SpeakerTurn` and `DiarError`

```rust
pub struct SpeakerTurn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker_id: u32,
}

#[derive(Debug, Error)]
pub enum DiarError {
    #[error("audio file not found: {0}")]
    AudioNotFound(PathBuf),
    #[error("invalid audio: {0}")]
    InvalidAudio(String),
    #[error("model error: {0}")]
    Model(String),
    #[error("diarization failed: {0}")]
    Diarization(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
```

### `Segment` grows one field

```rust
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub language_detected: Option<String>,
    pub confidence: f32,
    pub is_partial: bool,
    pub speaker_id: Option<u32>,    // NEW. None = not diarized.
}
```

`schema_version` bumps to `2`. The added field is `Option`-typed so the JSON shape stays additive: deserializers expecting v1 ignore the new field; v2 deserializers default `None` if it's missing.

### JSON wire shape (binary stdout)

```json
{
  "schema_version": 2,
  "model": "ggml-large-v3-turbo-q5_0",
  "audio_path": "tests/fixtures/audio/multi_speaker_60s.wav",
  "audio_duration_ms": 60000,
  "diarized": true,
  "segments": [
    {
      "start_ms": 0,
      "end_ms": 5200,
      "text": "Hello, this is the first speaker talking.",
      "language_detected": "en",
      "confidence": 0.92,
      "is_partial": false,
      "speaker_id": 0
    }
  ]
}
```

`diarized: bool` is a top-level field that says whether the diarizer ran (true) or was suppressed by `--no-diarize` (false). Cleaner than guessing from segment fields.

### CLI surface

```
panops-engine <wav> [--model <path>] [--language <code>] [--no-diarize]

New flag:
  --no-diarize    Skip the diarization pass. ASR-only output.
                  Faster; segments have speaker_id = null.
```

Default behavior changes: diarization is **on** by default. Slice 02 binaries that didn't pass `--no-diarize` will diarize; same JSON shape, plus `speaker_id` populated. CI tests opt out via `--no-diarize` for the ASR-only conformance and opt in for the diarizer conformance.

## Crate layout

New code in existing crates (no new crates):

- `crates/panops-core/src/diar.rs` (NEW): `Diarizer`, `SpeakerTurn`, `DiarError`.
- `crates/panops-core/src/segment.rs`: add `speaker_id` field, bump `SCHEMA_VERSION` to `2`, add `diarized` to `Transcript`.
- `crates/panops-core/src/conformance/diar.rs` (NEW): `run_suite(provider, fixtures_dir)` for `Diarizer`.
- `crates/panops-core/src/conformance/fakes.rs`: add `KnownTurnsFake` (a `Diarizer` fake that reads a sidecar `*.turns.json`).
- `crates/panops-core/src/lib.rs`: `pub mod diar;` re-exports.
- `crates/panops-portable/src/sherpa_diarizer.rs` (NEW): `SherpaDiarizer` impl.
- `crates/panops-portable/src/model.rs`: registry refactor (see below).
- `crates/panops-portable/src/lib.rs`: `pub use sherpa_diarizer::SherpaDiarizer;`.
- `crates/panops-portable/Cargo.toml`: add `sherpa-rs = "0.6"` (or current version at impl time).
- `crates/panops-portable/tests/conformance_diar.rs` (NEW): real `SherpaDiarizer` against the multi-speaker fixture.
- `crates/panops-engine/src/main.rs`: add `--no-diarize` flag, wire diarization, merge segments.
- `tests/fixtures/audio/multi_speaker_60s.{wav,transcript.txt,turns.json}` (NEW): three TTS turns, two voices (A-B-A pattern).
- `tests/fixtures/scripts/generate.sh`: append the multi-speaker generation step.

## Model registry refactor

`crates/panops-portable/src/model.rs` grows from one hardcoded model to a registry:

```rust
pub struct ModelInfo {
    pub name: &'static str,           // e.g., "ggml-large-v3-turbo-q5_0"
    pub url: &'static str,
    pub sha256: &'static str,
    pub approx_size_mb: u32,
}

pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        name: "ggml-tiny-q5_1",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin",
        sha256: "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
        approx_size_mb: 31,
    },
    ModelInfo {
        name: "ggml-base-q5_1",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin",
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        approx_size_mb: 57,
    },
    ModelInfo {
        name: "ggml-large-v3-turbo-q5_0",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        approx_size_mb: 547,
    },
];

pub const DEFAULT_MODEL_NAME: &str = "ggml-large-v3-turbo-q5_0";

pub fn ensure_model_by_name(name: &str, dest: &Path) -> Result<PathBuf, AsrError>;
pub fn default_model_path() -> Result<PathBuf, AsrError>;  // resolves DEFAULT_MODEL_NAME
```

Plus a separate registry for diarization models:

```rust
pub const DIAR_MODELS: &[ModelInfo] = &[
    ModelInfo {
        name: "sherpa-pyannote-segmentation-3-0",
        // Tarball; ensure_diar_model unpacks it next to itself before returning the .onnx path.
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
        sha256: "24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488",
        approx_size_mb: 7,
    },
    ModelInfo {
        name: "3dspeaker-eres2net-base",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx",
        sha256: "1a331345f04805badbb495c775a6ddffcdd1a732567d5ec8b3d5749e3c7a5e4b",
        approx_size_mb: 38,
    },
];
```

The `SherpaDiarizer::new()` takes paths to both ONNX files; a helper `default_diar_paths()` resolves them via the same `directories` data dir.

Sha256 values are computed once during implementation (Task 0) and baked in. CI verifies them just like the Whisper model.

## CI strategy: production / test split

The engine binary's `default_model_path()` resolves to `large-v3-turbo-q5_0` when no env override is set. CI overrides via `PANOPS_MODEL=<path-to-base-q5_1>` and the workflow's "Ensure model" step downloads `base.q5_1` (not `large-v3-turbo`).

Why `base.q5_1` for CI rather than keeping `tiny.q5_1`:
- WER threshold gets meaningful headroom (issue #9 documented the 3.5% margin pain on tiny).
- Still small enough to download in <10 s.
- Real model behavior, not toy-model behavior.

WER threshold drops from 0.35 to ~0.15 (verified at impl time on the actual fixture). Headroom target: ≥ 5%.

Diarization weights (~28 MB total) get the same cache-then-download treatment as Whisper. The CI cache key incorporates the `model.rs` hash so a registry change invalidates cleanly.

## Multi-speaker fixture

`tests/fixtures/audio/multi_speaker_60s.wav` — 60 s, three turns:

| Turn | Range | Voice (`say -v`) | Speaker | Sample text |
|---|---|---|---|---|
| 1 | 0–20 s | Samantha (en) | A | "Welcome to this meeting. Let's go over the agenda for today." |
| 2 | 20–40 s | Daniel (en, British male) | B | "Thanks. The first item is the budget review for next quarter." |
| 3 | 40–60 s | Samantha (en) | A (returning) | "Right. I'll start with the marketing line items, then move to engineering." |

Generated by `scripts/generate.sh` via three `say -o` calls (different `-v`) + `ffmpeg concat`. Voice A returns in turn 3 to test speaker re-identification (the genuinely-hard diarization case).

Sidecar `multi_speaker_60s.turns.json`:

```json
[
  {"start_ms": 0, "end_ms": 20000, "speaker_id": 0},
  {"start_ms": 20000, "end_ms": 40000, "speaker_id": 1},
  {"start_ms": 40000, "end_ms": 60000, "speaker_id": 0}
]
```

The fake `KnownTurnsFake` reads this file. The real `SherpaDiarizer` runs sherpa-onnx and we compare structurally.

## Conformance harness for `Diarizer`

`panops_core::conformance::diar::run_suite(provider, fixtures_dir)` runs:

For `multi_speaker_60s.wav`:
1. Returns ≥ 2 distinct `speaker_id` values across all turns.
2. Total covered duration ≥ 80% of audio duration (allowing for short silences).
3. For each consecutive turn pair: `turn[i].end_ms <= turn[i+1].start_ms` (ordered, non-overlapping).
4. **Re-identification structural check (real adapter only, skipped for fakes):** the speaker assigned to the largest portion of `0–20 s` and the speaker assigned to the largest portion of `40–60 s` are the **same** id. (Speaker A returns.)
5. Real adapter only: ≥ 1 turn intersects each ground-truth turn from `multi_speaker_60s.turns.json` for ≥ 50% of its duration. (Per-turn coverage; not strict ARI/DER yet.)

Loose. Tightens in a follow-up issue once we know what real sherpa-onnx output looks like on this fixture.

## ASR conformance update

The existing `asr::run_suite` continues to assert structural + WER on the EN/ES/mixed fixtures. Adds:
- A `multi_speaker_60s` case asserting structural ASR works (segments cover the audio, language detected, etc.) — no WER threshold (multi-voice TTS is harder than the single-voice fixtures, would push WER too high).

## Engine merge logic

`panops-engine`'s `run()` after slice 03:

```rust
let segments = asr.transcribe_full(&audio_path, lang_hint)?;
let segments = if args.no_diarize {
    segments
} else {
    let turns = diar.diarize(&audio_path)?;
    merge_speaker_turns(segments, &turns)
};
```

`merge_speaker_turns` is a small function in `panops-core::segment` (or a new `merge.rs`):

```rust
pub fn merge_speaker_turns(
    segments: Vec<Segment>,
    turns: &[SpeakerTurn],
) -> Vec<Segment> {
    segments.into_iter().map(|mut seg| {
        seg.speaker_id = dominant_speaker(seg.start_ms, seg.end_ms, turns);
        seg
    }).collect()
}
```

`dominant_speaker` returns the speaker who covers the most milliseconds of the segment, or `None` if no turn overlaps. Unit-tested with synthetic segments + turns.

## Risks

- **`sherpa-rs` build complexity.** Like `whisper-rs`, it pulls a C++ dependency (sherpa-onnx) via cmake. First build is slow. CI cmake build for both whisper.cpp + sherpa-onnx pushes the cold-cache CI runtime to ~10 min. Acceptable; warm cache stays fast. If it bites, swap to a pure-Rust diarizer (writing one over `ort` is a research project — defer).
- **Linux SIMD issue (issue #7) extends to sherpa-onnx.** Same `-march=native` cross-CPU SIGILL risk. If it fires, Linux CI stays compile-only for diarization tests too. Document in the same issue.
- **Model registry sha256 churn.** Three Whisper models + two diarization models = 5 hashes to maintain. If HF or k2-fsa re-upload, all break. Issue #8 (mirror to project-controlled host) becomes more urgent — track but don't block.
- **Speaker re-identification accuracy.** sherpa-onnx with the 3dspeaker eres2net model on TTS audio (which has artificially clean voice characteristics) might over-cluster (one speaker per turn) or under-cluster (everything is speaker 0). The conformance assertion is loose precisely because of this; if it fails on the first CI run, we either tune sherpa params or weaken the assertion. Don't lower below "≥ 2 distinct speakers" without flagging.

## Decisions locked

1. **`Diarizer` is its own port**, not a method on `AsrProvider`. Stateless. Returns `Vec<SpeakerTurn>`. Engine merges into segments by timestamp overlap.
2. **`Segment` grows `speaker_id: Option<u32>`.** `Transcript.schema_version` bumps to 2. Added field is optional so v1 consumers don't break.
3. **`Transcript` grows `diarized: bool`** at the top level. Makes "was the diarizer run?" explicit, not inferred.
4. **Library: `sherpa-rs`**, not hand-rolled `ort`. Wraps sherpa-onnx, gives us pyannote segmentation + speaker embedding + clustering as a single pipeline. Refines the master design's "pyannote via ort" — sherpa-onnx uses ONNX Runtime under the hood.
5. **Diarization models:** `sherpa-onnx-pyannote-segmentation-3-0` (~7 MB tarball, contains the segmentation `.onnx`) + `3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx` (~38 MB embedding model). Total ~45 MB. Downloaded on first run from k2-fsa/sherpa-onnx GitHub releases. URLs and sha256s baked into the registry above.
6. **Whisper production model bump: `large-v3-turbo-q5_0`** (~547 MB). Slice 02's `tiny.q5_1` becomes the demo / debug model.
7. **CI model: `base.q5_1`** (~57 MB) via `PANOPS_MODEL` override. Closes issue #9. WER threshold drops to ~0.15 (verified at impl time, ≥ 5% headroom required).
8. **Engine default: `--diarize` is implicit.** `--no-diarize` opts out. Reflects "post-pass = ASR + diarization" semantic from the master design.
9. **Multi-speaker fixture: `multi_speaker_60s.wav`** with three TTS turns (Samantha → Daniel → Samantha). A-B-A pattern tests speaker re-identification.
10. **Conformance for `Diarizer`: structural only.** Distinct count, coverage, ordering, re-identification of A in turns 1+3. ARI/DER metrics deferred.
11. **Slice scope stays scoped.** Rolling-during-recording orchestrator is slice 06+. Ports designed stateless / chunk-friendly so slice 06 can wire either at-end or rolling without changes here.

## Open questions / followups (NOT blocking slice 03)

- Mac-native diarization adapter using FluidAudio's CoreML diarizer (slice 06's `panops-mac` crate).
- Real ARI/DER metrics with tighter thresholds (when we have a non-TTS fixture or real meeting recordings).
- Streaming diarization (incremental speaker assignment as audio comes in). sherpa-onnx supports VAD + per-window speaker ID — the streaming pieces exist, the orchestrator doesn't.
- Speaker enrollment / persistent speaker IDs across meetings. ("This is Fran, this is Ada"). Different system; deferred.
- Reconciliation between live ASR transcript and post-pass transcript. Slice 06+ when live exists.

## Slice budget

- ~700 LOC across the new files (Diarizer trait + adapter + harness + fixture + engine merge logic).
- ~5 min cold CI runtime added (sherpa-onnx cmake build + diar models download ~45 MB + diar inference). Warm cache: +30 s.
- 3 Whisper model entries + 2 diarization model entries in the new registry, total ~648 MB of artifacts the production user pulls on first run (CI continues to pull ~57 MB via the `base.q5_1` override).
- One new fixture (`multi_speaker_60s.{wav,transcript.txt,turns.json}`).
- New deps: `sherpa-rs`, `tar`, `bzip2` (the last two for unpacking the segmentation tarball).

---

Ready to invoke writing-plans. Approve to proceed?
