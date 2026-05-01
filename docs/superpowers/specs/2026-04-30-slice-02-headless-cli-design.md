---
project: panops
date: 2026-04-30
status: design (pre-implementation)
slice: 02
authors: Fran Matzkin
type: design
---

# Slice 02 — Headless CLI on fixture audio

## Summary

The walking skeleton: feed a fixture WAV to the engine, get JSON segments back. Establishes the `AsrProvider` port, the portable `whisper-rs` adapter, the conformance harness pattern, and the JSON contract that every later slice (notes, IPC, Mac shell) consumes.

Multilingual from day one. The `panops-engine` binary is a developer/CI surface only — the SwiftUI Mac app is the actual product, and the binary exists for OSS portability and as the engine the future IPC server (slice 05) will host.

## Goals

- First slice that actually transcribes audio. End-to-end: WAV → segments → JSON.
- Land the `AsrProvider` trait + `Segment` domain type in `panops-core` (born when needed, not pre-traited).
- Land `panops-portable` with the `whisper-rs` adapter (real impl).
- Land a fake adapter under `tests/fixtures/` (reads the `*.transcript.txt` sidecar files committed in slice 01).
- Land the conformance harness pattern: both real and fake adapters pass the same test suite.
- Multilingual: EN, ES, and mixed fixtures all transcribe via Whisper auto-detect; per-segment `language_detected` populated.
- `panops-engine` binary: minimal dev/CI surface (`panops-engine <wav> [--model <path>]`).

## Non-goals

- Live streaming / partial segments. `is_partial` is always `false` in slice 02 (kept in the schema for forward compat).
- Subcommands or polished CLI UX. The binary's `--help` says "dev/debug tool — see Panops.app for the actual product."
- Multiple output formats (SRT/VTT/TXT). JSON only. The other formats arrive in a later slice when a real consumer needs them.
- Diarization, post-pass quality model, screenshots — those are slices 03 / 04.
- Mac-specific code. Slice 02 is target-os-agnostic; the `panops-mac` crate doesn't get touched.

## Architecture

```
┌──────────────────────────────────────┐
│       panops-engine (binary)         │
│  CLI: parse args, dispatch, emit     │
│  JSON to stdout. Dev/CI surface.     │
└──────────────┬───────────────────────┘
               │ depends on
               ▼
┌──────────────────────────────────────┐    ┌────────────────────────────┐
│          panops-portable             │───▶│         panops-core         │
│  WhisperRsAsr (impl AsrProvider)     │    │  trait AsrProvider          │
│  Wraps whisper-rs 0.16.0.            │    │  struct Segment             │
└──────────────────────────────────────┘    │  struct Transcript          │
                                            │  pub mod conformance::asr   │
┌──────────────────────────────────────┐    │  (test harness, runs fixtures
│  tests/fixtures/fake_adapter         │───▶│   against any impl).        │
│  TranscriptFileFake (impl AsrProv.)  │    └────────────────────────────┘
│  Reads *.transcript.txt sidecar.     │
└──────────────────────────────────────┘
```

## Public contracts

### `AsrProvider` trait (`panops-core`)

```rust
pub trait AsrProvider {
    /// Transcribe a complete audio file. Blocking, file-based.
    /// `language_hint` is `None` for auto-detect, `Some("en")` etc. to force.
    fn transcribe_full(
        &self,
        audio_path: &Path,
        language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError>;
}
```

`transcribe_window` (live streaming) is deliberately omitted. It lands in slice 06/07 when live capture exists.

### `Segment` and `Transcript` types

```rust
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub language_detected: Option<String>,  // ISO 639-1, None if undetermined
    pub confidence: f32,                    // [0.0, 1.0]
    pub is_partial: bool,                   // always false in slice 02
}

pub struct Transcript {
    pub segments: Vec<Segment>,
    pub model: String,                      // e.g., "ggml-tiny-q5_1"
    pub audio_path: PathBuf,
    pub audio_duration_ms: u64,
}
```

### JSON wire shape (binary stdout)

```json
{
  "schema_version": 1,
  "model": "ggml-tiny-q5_1",
  "audio_path": "tests/fixtures/audio/en_30s.wav",
  "audio_duration_ms": 26944,
  "segments": [
    {
      "start_ms": 0,
      "end_ms": 4520,
      "text": "The quick brown fox jumps over the lazy dog.",
      "language_detected": "en",
      "confidence": 0.91,
      "is_partial": false
    }
  ]
}
```

Serialized via `serde` derives on `Transcript` and `Segment`. Field naming uses `snake_case` consistently (no `serde(rename_all)` aliasing).

### CLI surface

```
panops-engine <wav> [--model <path>] [--language <code>]

Args:
  <wav>             Path to a 16 kHz mono WAV file.

Flags:
  --model <path>    Override the default model location. Defaults to
                    <data_dir>/panops/models/ggml-tiny-q5_1.bin, downloaded
                    on first run if absent.
  --language <code> ISO 639-1 hint. Default: auto-detect per segment.

Output: JSON Transcript on stdout. Diagnostics on stderr.
Exit codes: 0 success, 1 input error, 2 transcription error, 3 model error.
```

`--help` includes the line "Dev/CI tool. See https://github.com/vfmatzkin/panops for the desktop app."

## Crate layout

New workspace members:

- `crates/panops-portable/` — library crate.
  - `Cargo.toml`: depends on `panops-core`, `whisper-rs = "0.16"`, `hound` (WAV reader), `directories = "5"`, `reqwest` (download), `sha2` (checksum).
  - `src/lib.rs`: `pub struct WhisperRsAsr { model_path: PathBuf, ctx: WhisperContext }`. `impl AsrProvider for WhisperRsAsr`.
  - `src/model.rs`: `fn ensure_model(name: &str, dest: &Path) -> Result<()>`. Idempotent download, sha256 verify.
  - `tests/conformance.rs`: instantiates `WhisperRsAsr`, calls `panops_core::conformance::asr::run_suite`.
  - Marked `#[cfg(not(target_arch = "wasm32"))]` on the model fetcher; not relevant yet but documents intent.

- `crates/panops-engine/` — binary crate.
  - `Cargo.toml`: depends on `panops-core`, `panops-portable`, `clap = { version = "4", features = ["derive"] }`, `serde_json`.
  - `src/main.rs`: clap-derived CLI, dispatches to `WhisperRsAsr::transcribe_full`, emits JSON.

`panops-core` grows:
- `src/lib.rs`: re-exports.
- `src/segment.rs`: `Segment` + `Transcript`.
- `src/asr.rs`: `AsrProvider` trait + `AsrError` enum.
- `src/conformance/mod.rs`: `pub mod asr;` (only public test-helper module so far).
- `src/conformance/asr.rs`: `run_suite(&impl AsrProvider, fixtures_dir: &Path)`.

`tests/fixtures/fake_adapter/` is **not** a workspace crate. It's a tiny module under `panops-core`'s `dev-dependencies` path:
- `panops-core/src/conformance/fakes.rs`: `pub struct TranscriptFileFake;` reads the sibling `*.transcript.txt`, returns a single `Segment` covering the whole audio.
- `tests/conformance_fake.rs` in `panops-core`: instantiates `TranscriptFileFake`, runs the conformance suite.

Rationale: keeping the fake inside `panops-core` (behind a `pub` module that's only useful at test-time) avoids a separate workspace crate for one tiny struct. The `dev-dependencies` ergonomic remains intact.

## Model handling

Default model: `ggml-tiny-q5_1.bin` (~31 MB, multilingual quantized).

Resolution order:
1. `--model <path>` flag, if provided.
2. `PANOPS_MODEL` env var (escape hatch for CI), if set.
3. `<data_dir>/panops/models/ggml-tiny-q5_1.bin`, where `data_dir` is from `directories::ProjectDirs::from("dev", "panops", "panops").data_dir()`.
   - macOS: `~/Library/Application Support/panops/models/`
   - Linux: `~/.local/share/panops/models/`
   - Windows: `%APPDATA%\panops\panops\data\models\`

If absent at the resolved path, download from `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin`. Verify sha256 against a hardcoded constant. Stderr message: `Downloading ggml-tiny-q5_1.bin (31 MB)... done.`. No interactive prompt — silent acceptance, since the binary is non-interactive by design.

CI: GitHub Actions cache step keys on the model filename. Cache path: `~/.cache/panops/models/`. Override `PANOPS_MODEL=~/.cache/panops/models/ggml-tiny-q5_1.bin` in the workflow so tests use the cached file without touching the platform data dir.

## Fake adapter

`TranscriptFileFake` reads `<audio_path>.transcript.txt` (sibling file) and returns a single `Segment` with:
- `start_ms = 0`
- `end_ms = audio_duration_ms` (computed via `hound` from the WAV header)
- `text = the full transcript file contents, trimmed`
- `language_detected = Some(<derived from filename prefix>)`:
  - `en_*` → `"en"`
  - `es_*` → `"es"`
  - `mixed_*` → `"en"` (arbitrary; the mixed-fixture conformance assertion accepts en or es)
  - anything else → `None`
- `confidence = 1.0`
- `is_partial = false`

The fake does **not** parse the bracketed `[en, 0.0s..27.0s]` markers in the mixed transcript. That heuristic is real-adapter territory.

## Conformance harness

`panops_core::conformance::asr::run_suite(provider: &impl AsrProvider, fixtures_dir: &Path)`:

For each fixture in a hardcoded list `[en_30s, es_30s, mixed_60s]`:
1. Locate `<fixtures_dir>/audio/<name>.wav` and `<fixtures_dir>/audio/<name>.transcript.txt`.
2. Call `provider.transcribe_full(&audio_path, None)`.
3. Assert the contract:
   - `segments.len() >= 1`
   - For each segment: `start_ms <= end_ms <= audio_duration_ms + 100ms` (slack for rounding).
   - For consecutive segments `s[i]` and `s[i+1]`: `s[i].end_ms <= s[i+1].start_ms` (non-overlapping, ordered).
   - At least one segment has `language_detected.is_some()`.
4. Per-fixture language assertion:
   - `en_30s`: at least one segment has `language_detected == Some("en")`.
   - `es_30s`: at least one segment has `language_detected == Some("es")`.
   - `mixed_60s`: at least one segment has `language_detected.as_deref() == Some("en") || Some("es")`.
5. WER assertion (skipped if `provider.is_fake()` returns true — see below):
   - `en_30s`: WER between concatenated segment text and `en_30s.transcript.txt` ground truth ≤ 0.30.
   - `es_30s`: same, ≤ 0.30.
   - `mixed_60s`: no WER threshold; the auto-detect transcript is too unstable to gate on.

`AsrProvider` adds an optional `fn is_fake(&self) -> bool { false }` so fakes opt out of WER (their text is the transcript by construction; WER is trivially 0). Slight smell, but cleaner than two suite functions.

WER computation: a small in-tree implementation (`Levenshtein on whitespace-tokenized lowercase strings, ignoring punctuation`). No new crate dep.

## Build, test, CI

- `cargo build --workspace` — builds all four crates.
- `cargo test --workspace` — runs:
  - `panops-core`: unit tests + `tests/conformance_fake.rs` (fake adapter passes the suite).
  - `panops-portable`: `tests/conformance.rs` (real adapter passes the suite, including WER assertions). Requires the model.
  - `panops-engine`: smoke test invoking the binary on `en_30s.wav` and asserting the output JSON parses + matches the schema.
- CI workflow update: add a "fetch model" step before `cargo test` for the matrix jobs:
  ```yaml
  - name: Cache Whisper model
    uses: actions/cache@v4
    with:
      path: ~/.cache/panops/models
      key: ggml-tiny-q5_1-v1
  - name: Ensure model
    run: |
      mkdir -p ~/.cache/panops/models
      [ -f ~/.cache/panops/models/ggml-tiny-q5_1.bin ] || \
        curl -fL -o ~/.cache/panops/models/ggml-tiny-q5_1.bin \
          https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin
    env:
      PANOPS_MODEL: ~/.cache/panops/models/ggml-tiny-q5_1.bin
  ```
  `PANOPS_MODEL` is then propagated into the test step's env.

## Performance budget

On a clean GitHub Actions runner (no GPU), tiny.q5_1 transcribes 30 s of audio in roughly 5–15 s of CPU time. The conformance suite's three fixtures total ~110 s of audio, so the worst-case test pass is roughly 30–60 s. Acceptable for a per-PR check.

## Risks

- **Tiny model + synthetic TTS = noisy transcripts.** The 0.30 WER threshold is generous to cover this. If reality is even worse, we either bump the threshold (loosens the gate) or swap to base.q5_1 (~57 MB, slower but accurate). Decision deferred to first CI run.
- **Hugging Face URL availability.** Mirror via R2 / GitHub release asset if the HF URL is rate-limited or removed. Out of scope for slice 02; revisit if it bites.
- **Cold-cache CI.** First CI run after `actions/cache` invalidation pulls 31 MB. Tolerable.

## Decisions locked

1. Model: `ggml-tiny-q5_1.bin` (multilingual quantized, ~31 MB), downloaded on first run.
2. Cache path: cross-platform via `directories` crate. CI uses `~/.cache/panops/models/` via `PANOPS_MODEL` env.
3. Binary surface: `panops-engine <wav> [--model <path>] [--language <code>]`. Dev/CI only, no subcommands.
4. JSON shape: wrapper object with `schema_version`, `model`, `audio_path`, `audio_duration_ms`, `segments[]`. Timestamps in ms.
5. `is_partial` field present in schema, always `false` in slice 02.
6. New crates: `panops-portable`, `panops-engine`. `panops-core` grows the trait + types + conformance harness module.
7. Fake adapter lives inside `panops-core::conformance::fakes`, single-segment per audio file, language inferred from filename.
8. WER threshold: 0.30 on EN and ES fixtures via Levenshtein on whitespace-tokenized lowercase. Skipped for fakes via `is_fake()` opt-out.
9. CLI emits JSON only. SRT/VTT/TXT exporters land in a later slice when a consumer demands them.

## Open questions / followups (NOT blocking slice 02)

- Mirror the HF model URL on a project-controlled host before v0.1 release.
- Pick whether `confidence` is the avg of token log-probs or the segment-level `no_speech_prob` complement. (whisper-rs exposes both; we'll just pick one in implementation and note it.)
- Decide whether to expose `--no-download` flag for CI air-gapped scenarios. Likely yes once we hit one.

## Model selection UX — deferred to the desktop app

Slice 02 hardcodes `ggml-tiny-q5_1` because the binary is a dev/CI surface. End users never see this default — the desktop app does the picking. The model-picker UX lands later (likely riding slice 06 SwiftUI shell) and mirrors the LLM-provider picker pattern locked in the master design spec (Decision #2):

1. **First-run probe** via a new `ModelProbe` port. Inspects machine class (Apple Silicon yes/no, RAM, ANE) and pre-selects a default:
   - Apple Silicon + ≥16 GB RAM → `large-v3-turbo` (via WhisperKit + ANE on Mac).
   - Apple Silicon + 8 GB RAM → `medium.q5_1`.
   - Intel Mac or Linux → `base.q5_1`.
   - Probe fails / unknown → `tiny.q5_1`.
2. **First-run prompt** dialog with the detected default highlighted ("Recommended for your Mac"). User can accept or pick another. Detected default is a fallback, never used silently.
3. **Settings panel** lists models with quality/size/speed badges and a per-entry Download button. Mirrors the LLM provider settings UI.
4. **Per-meeting override** on the record sheet for one-off "use a heavier model this time" cases.

For slice 02 we just lock the contract that the engine accepts an arbitrary model path via `--model` (or `PANOPS_MODEL` env). The desktop app will call the binary with the user's chosen path. No engine-side picker UI in scope here.

`ModelProbe` itself is a new port that lands when slice 06's settings panel needs it. Slice 02 does not introduce it.

---

Ready to invoke writing-plans on this. Approve to proceed?
