# Slice 10 — WhisperKit ASR Sidecar (Anchor A continuation)

**Status:** Locked design. Open for plan-writing.
**Date:** 2026-05-12
**Author:** Franco Matzkin (Claude wrote the draft autonomously while the maintainer was away; spec is open for maintainer revision in the PR.)
**Predecessor:** [slice 09 design](2026-05-11-slice-09-mac-shell-walking-skeleton-design.md)
**North-star tie-in:** Anchor A's second piece. Addresses the perf concern the maintainer raised mid-slice-08 ("the tool is getting slower and slower") by introducing the macOS-native WhisperKit ASR adapter. Replaces `whisper-rs` for macOS production runs; `whisper-rs` stays for CI's fast path and as the portable fallback.

## Problem

After slice 09, the Mac shell can drive the engine end-to-end — but the engine still uses `WhisperRsAsr` (`whisper.cpp` via `whisper-rs`) on CPU+BLAS. On Apple Silicon, that's 5–10× slower than the platform-native path. The maintainer flagged this multiple times across slice 07/08; slice 09's smoke against the public test fixture took ~5 minutes for a 30s clip via the GUI.

AGENTS.md anchors the fix as `apps/panops-asr-mac/` — a Swift sidecar process using `WhisperKit` (CoreML+Metal under the hood). The engine on macOS should pick this sidecar-backed `AsrProvider` over `WhisperRsAsr` automatically.

## Goal

Ship the smallest viable WhisperKit ASR sidecar plus the Rust adapter that wraps it as an `AsrProvider`. The sidecar runs as a child process owned by the engine, communicates via newline-delimited JSON-RPC over stdio, and serves a single method: `asr.transcribe`. The engine prefers `WhisperKitAsr` when the sidecar binary is available on macOS and falls back to `WhisperRsAsr` otherwise.

No live capture, no streaming, no progress events. Slice 11+ is where streaming + the LLM sidecar land. Anchor B (live capture) is risk-last.

## Decisions

| # | Decision | Reason |
|---|---|---|
| D1 | **New top-level Rust crate `crates/panops-mac/`**, `#[cfg(target_os = "macos")]`, holds `WhisperKitAsr` and any future macOS-native adapters (FoundationModels LLM, ScreenCaptureKit later) | Aligns with the AGENTS.md repo-conventions line that already names this crate. Keeps `panops-portable` portable; macOS-specific deps don't leak into the Linux build. |
| D2 | **New SwiftPM target at `apps/panops-asr-mac/`**, executable, depends on `argmaxinc/WhisperKit` | Matches AGENTS.md's `apps/panops-asr-mac/` anchor. Separate from `apps/Panops/` shell so build failures in one don't block the other. |
| D3 | **IPC protocol: newline-delimited JSON-RPC over the sidecar's stdin/stdout** | Simplest. No socket setup. Mirrors the `apps/Panops/` shell's hand-rolled JSON-RPC envelopes (`JsonRpcRequest<P>`, positional-array params). Sidecar is single-tenant; no need for the WS upgrade dance slice 09 had to hand-roll. |
| D4 | **Sidecar lifecycle: engine spawns on first ASR call, keeps alive for subsequent calls, kills on engine shutdown** | Model load (~3-5s) is the dominant cost; reusing across calls amortizes it. Same pattern as slice 09's `EngineProcess` (engine→child) but inverted (engine is parent now). |
| D5 | **WhisperKit model: `openai_whisper-tiny`** by default; configurable via env var | Matches the CI optimization choice from PR #121 — tiny is good enough for the bilingual recursion baseline tests; recall trade-off acceptable for v0.1. WhisperKit downloads the model on first run from HuggingFace. |
| D6 | **Sidecar binary discovery: `PANOPS_ASR_SIDECAR_BIN` env var (dev escape hatch) with `Bundle.main.bundleURL/Contents/Resources/panops-asr-mac` fallback for production**. Matches slice 09's `PANOPS_ENGINE_BIN` pattern | AGENTS.md forbids user-facing env vars but explicitly allows dev/CI escape hatches when flagged. Production builds resolve via app bundle. |
| D7 | **Adapter resolution: engine picks `WhisperKitAsr` on macOS when `PANOPS_ASR_SIDECAR_BIN` is set AND the binary is executable; otherwise falls back to `WhisperRsAsr`** | Walking-skeleton stance: existing `whisper-rs` path stays as default. Sidecar is opt-in via env var until slice 12 bundles it. CI's fast job sets `PANOPS_SKIP_HEAVY=1` so this codepath isn't exercised on PRs. |
| D8 | **Conformance: `WhisperKitAsr` passes the existing `panops-core` `AsrProvider` conformance suite** on macOS via a new `conformance_whisperkit.rs` test gated on `PANOPS_ASR_SIDECAR_BIN` + `PANOPS_SKIP_HEAVY` | Same gating pattern as `conformance_real.rs` (self-skips when env var missing). Heavy-test job sets the env var to exercise the new adapter. |
| D9 | **AsrProvider trait unchanged** | Slice 07 just reshaped it (samples-based). Touching the trait again is the slice-08 anti-pattern restated. The sidecar's JSON-RPC method must accept the same `(samples, sample_rate, language_hint)` shape on the wire. |
| D10 | **Audio transport over JSON-RPC: bytes encoded as a path to a temp WAV file on disk** (NOT inline base64) | A 30s 16-kHz mono WAV is ~960 KB; base64 inflates to ~1.3 MB per request. Stdio JSON-RPC isn't built for binary blobs. Engine writes a temp WAV, passes path to sidecar, sidecar loads + decodes + transcribes, deletes temp file in finally. Trades a brief disk write for sane wire protocol. |

## Scope

### In

1. **New Rust crate** `crates/panops-mac/` with:
   - `src/lib.rs` (`#[cfg(target_os = "macos")]` module gates)
   - `src/whisperkit_asr.rs` — `WhisperKitAsr` struct implementing `AsrProvider`, manages child process + JSON-RPC over stdio
   - `Cargo.toml` — workspace member, depends on `panops-core`, `serde`, `serde_json`, `tempfile`
2. **Workspace Cargo.toml** — add `crates/panops-mac` to `members`.
3. **New SwiftPM target** `apps/panops-asr-mac/`:
   - `Package.swift` (depends on `https://github.com/argmaxinc/WhisperKit`)
   - `Sources/PanopsAsrMac/main.swift` — main loop: read JSON-RPC line on stdin → run WhisperKit → write JSON-RPC response on stdout
   - `Sources/PanopsAsrMac/Codecs.swift` — Codable structs matching `panops-protocol`'s wire types (`Segment`, `Transcript`, etc.)
   - `README.md` — dev setup
4. **Engine adapter resolution** in `crates/panops-engine/src/server/mod.rs` + `crates/panops-engine/src/main.rs`:
   - On macOS: if `PANOPS_ASR_SIDECAR_BIN` resolves to an executable, instantiate `WhisperKitAsr`; otherwise `WhisperRsAsr`.
   - On other targets: `WhisperRsAsr` only.
5. **CI heavy-test job** updated to download/build the sidecar binary and set `PANOPS_ASR_SIDECAR_BIN`. Runs the new conformance test.
6. **Conformance test** `crates/panops-mac/tests/conformance_whisperkit.rs` exercising the `AsrProvider` conformance suite against the spawned sidecar. Self-skips when env var unset.
7. **Dev README** at `apps/panops-asr-mac/README.md` documenting build + env-var setup for local engine runs.

### Out (filed as debt if surfaced)

- **Replace `WhisperRsAsr` entirely on macOS.** Keep both. Sidecar is opt-in this slice.
- **Streaming partial events** (`asr.partial`). Anchor B.
- **WhisperKit progress callbacks** surfaced over IPC. Future slice when progress events ship for the GUI.
- **LLM sidecar** (`apps/panops-llm-mac/` via FoundationModels). Slice 11+.
- **Custom Whisper model selection beyond tiny.** Hard-coded for slice 10; flag-driven future slice.
- **Engine packaging the sidecar in `.app/Contents/Resources/`.** Slice 12 (sign + notarize).
- **Hot-reload / health-check.** Engine restarts the sidecar on subsequent runs; no in-flight crash recovery this slice.

## Architecture

```
┌──────────────────────────────────────┐
│ panops-engine (Rust)                 │
│                                      │
│  Decides at startup:                 │
│   if macOS + PANOPS_ASR_SIDECAR_BIN  │
│     → Arc::new(WhisperKitAsr::new()) │
│   else                               │
│     → Arc::new(WhisperRsAsr::new())  │
│                                      │
│  ┌────────────────────────────────┐  │
│  │ WhisperKitAsr   (panops-mac)   │──┼──┐ spawn + stdio
│  │   - transcribe(samples,...)    │  │  │
│  │   - lazy spawn on first call   │  │  │
│  │   - keep alive across calls    │  │  │
│  └────────────────────────────────┘  │  │
└──────────────────────────────────────┘  │
                                          │
            ┌─────────────────────────────▼──────┐
            │ apps/panops-asr-mac (Swift)        │
            │                                    │
            │   stdin:  JSON-RPC request line    │
            │   stdout: JSON-RPC response line   │
            │                                    │
            │   WhisperKit instance, model       │
            │   loaded on first request,         │
            │   reused thereafter.               │
            └────────────────────────────────────┘
```

The Rust engine + Swift sidecar communicate over the child process's stdio. Each request is one line; each response is one line. The sidecar runs WhisperKit (CoreML + Metal on Apple Silicon).

## Components

### Rust side — `crates/panops-mac/`

`WhisperKitAsr`:

```rust
pub struct WhisperKitAsr {
    inner: Mutex<SidecarChild>,
    binary: PathBuf,
}

struct SidecarChild {
    process: Option<std::process::Child>,
    stdin: Option<std::process::ChildStdin>,
    stdout_lines: Option<std::io::Lines<BufReader<std::process::ChildStdout>>>,
    next_id: u64,
}

impl WhisperKitAsr {
    pub fn new(binary: PathBuf) -> Self { ... }
    fn ensure_spawned(&self, inner: &mut SidecarChild) -> Result<(), AsrError> { ... }
    fn send_request<P: Serialize, R: DeserializeOwned>(
        &self, method: &str, params: P,
    ) -> Result<R, AsrError> { ... }
}

impl AsrProvider for WhisperKitAsr {
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        // 1. Write samples to a temp WAV (16-kHz mono).
        // 2. Send JSON-RPC `asr.transcribe { audio: <temp_path>, sample_rate, language_hint }`.
        // 3. Parse response; build Transcript.
        // 4. Delete temp WAV.
    }
}
```

Lazy spawn: the child process starts on the FIRST `transcribe()` call. Drop impl on `WhisperKitAsr` SIGTERMs the child.

### Swift side — `apps/panops-asr-mac/`

`main.swift`:

```swift
import Foundation
import WhisperKit

let whisperKit = try await WhisperKit(
    model: ProcessInfo.processInfo.environment["PANOPS_WHISPERKIT_MODEL"] ?? "openai_whisper-tiny"
)

while let line = readLine() {
    let request = try JSONDecoder().decode(JsonRpcRequest<TranscribeParams>.self, from: Data(line.utf8))
    let result = try await whisperKit.transcribe(audioPath: request.params[0].audio)
    let response = JsonRpcResponse(id: request.id, result: SidecarTranscript(segments: result.segments.map(...)))
    let body = try JSONEncoder().encode(response)
    print(String(data: body, encoding: .utf8)!)
}
```

(Pseudocode — the real implementation handles errors, EOF, parse failures, etc.)

### Engine wiring — `crates/panops-engine/src/server/mod.rs` + `main.rs`

```rust
fn pick_asr(model_path: PathBuf) -> Result<Arc<dyn AsrProvider + Send + Sync>, String> {
    #[cfg(target_os = "macos")]
    if let Ok(bin) = std::env::var("PANOPS_ASR_SIDECAR_BIN") {
        let path = PathBuf::from(bin);
        if path.is_file() {
            // Note: WhisperKitAsr defers spawn to first use, so this
            // is a cheap construction (no IO).
            return Ok(Arc::new(panops_mac::WhisperKitAsr::new(path)));
        }
    }
    WhisperRsAsr::new(model_path)
        .map(|a| Arc::new(a) as Arc<dyn AsrProvider + Send + Sync>)
        .map_err(|e| e.to_string())
}
```

Both call sites (`serve` and the CLI `transcribe_with_vad`) use this resolver.

## Data flow (one request)

1. Engine pipeline reaches the per-region ASR call: `asr.transcribe(chunk, sr, hint)`.
2. `WhisperKitAsr::transcribe` writes `chunk` to a temp WAV under `std::env::temp_dir()`.
3. `WhisperKitAsr::send_request` builds JSON-RPC `{ "jsonrpc":"2.0","id":N,"method":"asr.transcribe","params":[{ "audio": "/tmp/...wav", "sample_rate": 16000, "language_hint": null }] }`, writes it + newline to the sidecar's stdin.
4. Engine blocks on a read from sidecar stdout (line-buffered).
5. Sidecar reads the line, decodes, calls `WhisperKit.transcribe(audioPath:)`, encodes response, writes line + flush.
6. Engine reads the response line, parses, returns the `Transcript`.
7. Engine deletes the temp WAV.

## Testing

### Unit (cheap, no WhisperKit)

`crates/panops-mac/src/whisperkit_asr.rs` mod-tests:

1. **temp WAV roundtrip** — `write_temp_wav(samples, sr)` produces a valid 16-kHz mono WAV that `hound` can re-read. Verifies the audio handoff path.
2. **JSON-RPC request shape** — encode a sample request, assert wire shape (positional array, snake_case keys, etc.).
3. **JSON-RPC response decode** — decode a known-good response into `Transcript`.

### Conformance (gated; runs in heavy-test)

`crates/panops-mac/tests/conformance_whisperkit.rs`:

- Self-skips if `PANOPS_ASR_SIDECAR_BIN` is unset (matches `conformance_real.rs` pattern).
- Otherwise: instantiates `WhisperKitAsr`, runs the `panops_core::conformance::asr::run_suite` harness against it. The same suite that `WhisperRsAsr` passes.

### CI

- Fast `test` job (no env var): conformance test self-skips. Unit tests run.
- `heavy-test` job: builds the sidecar (`cd apps/panops-asr-mac && swift build --configuration release`), exports `PANOPS_ASR_SIDECAR_BIN`, runs the conformance test.

## Three-tier boundaries

### ✅ Always do

- Run `cargo fmt && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` before each commit.
- Run `cd apps/panops-asr-mac && swift build && swift test` (if a Tests dir exists) before each commit on the Swift side.
- File a debt issue for any "deferred" item.
- Use `Bundle.main.bundleURL` for the production sidecar lookup; env var is dev escape hatch only.
- Commit per task in the plan.

### ⚠️ Ask first

- Adding a Rust dependency beyond `panops-core`, `serde`, `serde_json`, `tempfile`, `hound`.
- Adding a SwiftPM dependency beyond `WhisperKit` itself.
- Changing the JSON-RPC wire shape after first commit lands.
- Removing `WhisperRsAsr` from the engine's adapter resolution path.
- Changing the engine's behavior when `PANOPS_ASR_SIDECAR_BIN` is set but the binary doesn't exist (currently: fall back to whisper-rs; could become hard error).
- Bundling the sidecar in `.app/Contents/Resources/` (that's slice 12).

### 🚫 Never do

- Touch the `AsrProvider` trait shape this slice. Slice 07 reshaped it; slice 10 doesn't.
- Pre-trait a "Sidecar transport" abstraction. Hand-roll the stdio JSON-RPC inline; abstract later when there are 2+ sidecars (slice 11 brings the LLM sidecar — that's when DRY kicks in, not before).
- Stream audio over JSON-RPC. Use the temp-WAV path (D10).
- Drop the existing `whisper-rs` adapter or its CI fast-path tests.
- Add live-capture event types (`asr.partial`, `screenshot`). Anchor B.
- Phone home. Zero telemetry, ever.
- Auto-merge the PR.

## Acceptance criteria

1. `cargo build --workspace --locked` succeeds on macOS and Linux (panops-mac is `cfg(target_os = "macos")`).
2. `cargo test --workspace --locked` succeeds with `PANOPS_SKIP_HEAVY=1` and no sidecar env vars (conformance self-skip).
3. `cd apps/panops-asr-mac && swift build` succeeds on macOS 14+ with the Swift toolchain.
4. Running `apps/panops-asr-mac/.build/release/panops-asr-mac` and piping a known JSON-RPC `asr.transcribe` request on stdin returns a valid response with non-empty segments.
5. Conformance test `conformance_whisperkit::real_whisperkit_passes_conformance` passes with `PANOPS_ASR_SIDECAR_BIN` set + `PANOPS_SKIP_HEAVY` unset.
6. Manual smoke from repo root with `PANOPS_ASR_SIDECAR_BIN=$PWD/apps/panops-asr-mac/.build/release/panops-asr-mac` set, running `panops-engine notes <wav>` against the public `en_30s.wav` fixture → produces notes; manual measurement shows the macOS run is meaningfully faster than the `whisper-rs` baseline (the headline perf fix).
7. `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.
8. CI heavy-test job's new `conformance_whisperkit` step passes on macos-latest.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| WhisperKit's Swift API has changed since the spec author's training cutoff (model loading, transcribe signature) | High | Read `argmaxinc/WhisperKit`'s README + example code BEFORE writing the sidecar's main.swift. If API differs from spec pseudocode, update the spec via an Amendment section, then implement. |
| WhisperKit model download blocks offline / sandboxed runs (CI) | Medium | Document the first-run download. The heavy-test job has network; OK for CI. For local dev: document. |
| `WhisperKit` package requires Xcode (not just Command Line Tools) | Medium | Verify during the spike. If it does, document in README and add a CI check that ensures Xcode is available on macos-latest runners (which it is, by default). |
| Sidecar process orphans the engine (Rust binary leaks the child on panic) | Low | Use `std::process::Child` with explicit `Drop` impl that SIGTERMs. Same pattern as slice 09's `EngineProcess.swift`, in reverse. |
| Per-call latency overhead from temp-WAV write + JSON-RPC framing dominates the perf win | Low | 16-kHz mono WAV write of a 60s region is ~2 MB / ~10 ms. Negligible vs Whisper inference time (1-5s). Document the trade-off. |
| Tiny model recall is too low on bilingual real recordings via WhisperKit (different quantization than `ggml-tiny-q5_1`) | Medium | WhisperKit's `openai_whisper-tiny` is `ggml-tiny.bin` converted to CoreML. Quantization differs. If real-meeting smoke shows worse recall than `whisper-rs + tiny`, file as debt: bump to `openai_whisper-base` or `-small` for production. |
| WhisperKit lazily downloads models from HF to `~/Documents/huggingface/` by default — different cache path than `whisper-rs` | Low | Document. Engine doesn't need to know; sidecar handles its own cache. |

## Open questions (deferred to future slices)

1. **Per-segment language detection** — slice 07/08's confidence-recursion was implemented at the orchestration layer (`recursive_asr.rs`), above the adapter. WhisperKitAsr just answers `transcribe`. The recursion still works. Future slice: WhisperKit also has its own language probability surface; evaluate whether using its native signal beats the recursion heuristic.
2. **WhisperKit's diarization / VAD capabilities** — it has some built-in. The engine still uses sherpa-rs for diar and Silero VAD. Slice 11+ could collapse some of these into the sidecar.
3. **Auto-restart on sidecar crash** — slice 10 doesn't recover; user reruns the engine. Future slice: health-check + restart loop.
4. **Streaming partial events** — Anchor B (live capture).
5. **LLM sidecar** (`apps/panops-llm-mac/` via FoundationModels) — next slice in the Anchor A chain.
