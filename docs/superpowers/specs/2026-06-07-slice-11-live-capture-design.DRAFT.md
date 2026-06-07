# Slice 11 — Live Capture (Anchor B)

**Status:** **DRAFT — pending maintainer approval.** This is the design gate for #137. Nothing in `apps/panops-capture-mac/` or `crates/panops-mac/` is implemented until this is approved. Brainstorm: this file. Plan: forthcoming via `superpowers:writing-plans` after approval.

## Problem

North-star criterion #1 — *"Open the Mac app, hit record, run a real bilingual meeting → audio + screenshots captured"* — is unmet. Today the `Capture` port has a trait, a `FakeCapture`, a conformance suite, wire events, and handler wiring, but no real adapter: `capture_resolver::pick_capture()` returns `NotYetImplementedCapture` outside tests (`crates/panops-engine/src/capture_resolver.rs:46`). Live capture is **Anchor B** and the risk-last surface per the trajectory. This slice fills the gap.

## Goal

A macOS adapter that captures system audio + microphone + screenshots through a Swift ScreenCaptureKit sidecar, finalizes a single 16 kHz mono WAV plus deduplicated screenshot JPEGs into the meeting directory, and passes the existing `Capture` conformance suite. Completes **Anchor B**. No change to the `Capture` port contract.

## What already exists (scaffolding, do not rebuild)

This slice is narrower than it looks — the Rust side is mostly wired. Already shipped:

- **Port** — `crates/panops-core/src/capture.rs`: `Capture` trait (`start_capture(meeting_id, meeting_dir, config) -> CaptureSession`, `stop_capture(session) -> CaptureResult`), `CaptureConfig { audio_sources, screenshot_interval_ms = 500, screenshot_threshold = 0.15 }`, `AudioSources { SystemOnly, MicOnly, SystemAndMic }`, `CaptureResult { audio_path, screenshot_paths, duration_ms }`, `CaptureError`.
- **Fake** — `FakeCapture` (`conformance/fakes.rs:719`): synthetic 440 Hz sine WAV + fixture screenshots.
- **Conformance** — `conformance/capture.rs`: asserts a 16 kHz **mono** WAV, screenshot paths exist, `SessionNotFound` on unknown session, `is_fake` marker.
- **Resolver placeholder** — `capture_resolver.rs`: `OnceLock`-cached `pick_capture()`; `PANOPS_TEST_CAPTURE=1` → `FakeCapture`, else `NotYetImplementedCapture`.
- **Wire events** — `panops-protocol`: `Event::Screenshot(ScreenshotEvent { meeting_id, timestamp_ms, path })` and `Event::RecordingProgress(RecordingProgressEvent { meeting_id, bytes_captured, duration_ms })` (`methods.rs:80,89`). Defined; not yet emitted.
- **Handler wiring** — `server/handlers.rs`: `meeting.start` / `meeting.stop` already call `pick_capture()` → `start_capture` / `stop_capture`, with active sessions tracked in `Arc<Mutex<HashMap<String, CaptureSession>>>`. `MeetingStartParams` already carries `audio_sources` + screenshot knobs, mapped via `AudioSourcesWire`.
- **Storage** — `MeetingStore::create_screenshot` / `list_screenshots` with `ScreenshotDraft { meeting_id, timestamp_ms, path, feature_print: Option<Vec<u8>>, caption }`.
- **Prod resolution helper** — `sidecar_binary::sibling_of_engine(name)` (current-exe sibling) + `executable_file` validation.

**The gap this slice closes:** (1) the Swift `panops-capture-mac` sidecar, (2) the `ScreenCaptureKitCapture` Rust adapter, (3) the resolver real tier. Everything else is plumbing that already lands the result.

## Proposed decisions (for maintainer approval)

- **Swift sidecar `apps/panops-capture-mac`**, mirroring `apps/panops-asr-mac` for layout, stdio framing, and the spawn/`Drop` lifecycle. ScreenCaptureKit + AVFoundation do the capture; Vision does screenshot dedup.
- **Separate buffers, mixed output.** SCStream delivers system audio (`.audio`) and microphone (`.microphone`) as **separate** `CMSampleBuffer`s (macOS 26: `SCStreamConfiguration.capturesAudio` for system, `captureMicrophone` + `microphoneCaptureDeviceID` for mic). The sidecar resamples both to 16 kHz mono and **sums-and-clamps into one WAV** — exactly the input the ASR pipeline and conformance suite require.
- **`ScreenCaptureKitCapture` adapter** in `crates/panops-mac`, `impl Capture`, reusing the `whisperkit_asr.rs` spawn/stdio/`Drop` machinery — with one structural difference: capture is a **long-running session**, not request/response (see Architecture).
- **Resolver real tier** mirroring `asr_resolver::pick_asr`: `PANOPS_CAPTURE_SIDECAR_BIN` (dev/CI gate) → else `sibling_of_engine("panops-capture-mac")` (prod) → else the existing error path.
- **Batch-at-stop for v0.1** (recommended; see Open Questions §1). The sidecar accumulates audio + screenshots and returns the full `CaptureResult` at `stop_capture`. The live `Screenshot` / `RecordingProgress` events stay defined-but-unemitted until a follow-up wires them — north-star explicitly scopes live streaming UI out of v0.1 ("live partials are bonus, not required").
- **Port unchanged.** `start_capture` / `stop_capture` signatures, `CaptureConfig`, `CaptureResult` all stay as-is. `panops-core` stays platform-free; the adapter is `#[cfg(target_os="macos")]` in `panops-mac`.

## Scope

### In

- `apps/panops-capture-mac/` SwiftPM executable (`platforms: [.macOS(.v26)]`):
  - `Sources/PanopsCaptureMac/main.swift` — stdio JSON-lines control loop (mirrors the ASR `main.swift` readLine pattern), routing `capture.start` / `capture.stop`.
  - `Recorder.swift` — owns the `SCStream`, `SCStreamOutput` for `.audio` / `.microphone` / `.screen`, AVFoundation resample + mix, WAV writer.
  - `Screenshotter.swift` — frame sampling at `screenshot_interval_ms`, Vision `VNGenerateImageFeaturePrint` cosine-distance dedup against the last kept frame at `screenshot_threshold`, JPEG write.
  - `Codecs.swift` — control-line request/response + event Codables.
- `crates/panops-mac/src/screencapturekit_capture.rs` — `ScreenCaptureKitCapture` impl `Capture`; lazy spawn, stdio control framing, `Drop` closes stdin + reaps child, respawn on broken pipe. Reuses the framing discipline from `whisperkit_asr.rs` (16 MiB line cap, `jsonrpc` version check, id pairing for the start/stop responses).
- `crates/panops-engine/src/capture_resolver.rs` — real tier (`PANOPS_CAPTURE_SIDECAR_BIN` → `sibling_of_engine` → error), `#[cfg(target_os="macos")]`, replacing `NotYetImplementedCapture` in prod while keeping the `PANOPS_TEST_CAPTURE` fake tier for tests.
- Tests: `ScreenCaptureKitCapture` against a **fake sidecar binary** (a stub speaking the control protocol with canned responses) through the same `Capture` conformance suite; Swift unit tests for the mix + dedup math.

### Out (file as debt if surfaced)

- **Live event emission** (`Screenshot` / `RecordingProgress` over WebSocket) — deferred per §1; the wire types already exist, the emission path is a follow-up.
- **Separate per-source audio tracks for diarization** (system = remote, mic = local) — deferred per §2; would change `CaptureResult`.
- **Production `Bundle.main` sidecar resolution + signing/entitlements/notarization** — lands in the packaging slice; this slice keeps the `sibling_of_engine` + env gate.
- **Screenshot captioning** (`ScreenshotRow.caption`) — already marked future-slice in the store.
- **Per-meeting `meeting.db` segment/screenshot persistence during live capture** — only if not already covered by the storage wiring; confirm against the current handler before filing.

## Architecture

```
panops-core (Capture port; UNTOUCHED)
        ▲
        │ impl
crates/panops-engine/src/capture_resolver.rs ── pick_capture() ──┐
        │  PANOPS_CAPTURE_SIDECAR_BIN (dev) | sibling (prod)      │ else
        ▼                                                         ▼
crates/panops-mac/ScreenCaptureKitCapture ──spawn/stdio control──►  NotYetImplementedCapture (error)
        │
        ▼
apps/panops-capture-mac (Swift):
    main ↔ Recorder(SCStream: .audio + .microphone → resample/mix → 16k mono WAV)
         ↔ Screenshotter(SCStream frames → Vision FeaturePrint dedup → JPEG)
```

### The structural difference from ASR/LLM sidecars

WhisperKit and FoundationModels sidecars are **request/response**: one `transcribe` / `complete` line in, one result line out, id-paired. Capture is a **stateful session**:

1. `start_capture` spawns the sidecar (or reuses a live one), writes one `capture.start` control line `{ id, meeting_id, audio_path, screenshots_dir, audio_sources, screenshot_interval_ms, screenshot_threshold }`, and reads one `capture.started` ack. The sidecar then runs `SCStream` in the background, writing audio to the WAV and screenshots to disk. **No further lines flow until stop** (batch-at-stop).
2. `stop_capture` writes one `capture.stop` control line `{ id, meeting_id }`; the sidecar stops the stream, finalizes the WAV, and returns `{ audio_path, screenshot_paths, duration_ms }` as the response. The adapter maps that to `CaptureResult`.

So the adapter reuses the **spawn/stdio/`Drop`/respawn machinery** from `whisperkit_asr.rs` verbatim in shape, but the control protocol is start/ack + stop/result rather than a single paired call. The 16 MiB line cap, `deserialize_jsonrpc_version`, id-pairing, and `*slot = None` on stdio failure all carry over. Per AGENTS.md, any `OnceLock` / once-init slot the resolver exposes must reach a terminal state on every path (success, error, panic) — wrap heavy init in `catch_unwind` as precedent `02559a3` did.

## Control protocol (JSON-lines over stdio, mirrors ASR framing)

- `capture.start` params `{ meeting_id, audio_path, screenshots_dir, audio_sources: "system_only"|"mic_only"|"system_and_mic", screenshot_interval_ms, screenshot_threshold }` → result `{ started_at_ms }` or error → `CaptureError::{PermissionDenied, Sidecar, InvalidConfig}`.
- `capture.stop` params `{ meeting_id }` → result `{ audio_path, screenshot_paths: [..], duration_ms }` → `CaptureResult`. Unknown session → `CaptureError::SessionNotFound`.
- Errors map to `CaptureError` at the adapter boundary. Full sidecar detail (ScreenCaptureKit / TCC state) goes to **stderr** (Console.app); an opaque code-only message goes over the wire, matching the ASR sidecar's leak discipline.
- **(deferred, §1)** `screenshot` / `progress` JSON-RPC *notifications* (no id) for the live path — the line format is reserved now so wiring them later is additive, not breaking.

## Audio mixing

The ASR pipeline and the conformance suite both require a single **16 kHz mono** PCM WAV (`conformance/capture.rs:93` asserts `sample_rate == 16_000`, `channels == 1`). macOS 26 delivers system and mic audio as **separate** sample-buffer streams, so the sidecar must:

1. Receive `.audio` (system, typically 48 kHz stereo) and `.microphone` (device-native) buffers on the `SCStreamOutput` callback.
2. Resample each to 16 kHz mono via `AVAudioConverter`.
3. Mix per `audio_sources`: `SystemOnly` / `MicOnly` pass one stream through; `SystemAndMic` sums the two and clamps to `[-1, 1]` (sum-and-clamp, not average — average halves a single active speaker).
4. Append to the WAV (16-bit PCM) via the same `hound`-equivalent path the WAV writer uses.

Clock skew between the two streams is bounded by writing on the buffer presentation timestamps and zero-filling gaps; absolute A/V sync is not required (ASR is timestamp-tolerant within the segment).

## Screenshot sampling, dedup, time-anchoring

- **Sampling** — every `screenshot_interval_ms` (default 500), grab the latest `SCStream` video frame (or `SCScreenshotManager` for an on-demand grab; frame-tap preferred to avoid a second stream).
- **Dedup** — compute a Vision `VNGenerateImageFeaturePrintRequest` feature print, cosine-distance it against the last **kept** frame's print. Below `screenshot_threshold` (default 0.15) → drop as a near-duplicate; at/above → keep, write JPEG, update the reference print. This is the same FeaturePrint dedup the `CaptureConfig` knobs were designed around.
- **Time-anchoring** — each kept JPEG is named/recorded with `timestamp_ms` relative to capture start (the same clock as audio sample offsets), so notes generation can anchor a screenshot to the transcript segment active at that moment. The feature print is retained for the eventual `ScreenshotRow.feature_print` column.

## Permissions / TCC flow

- **Screen Recording** (TCC) — required for `SCStream`. The **sidecar process** runs ScreenCaptureKit, so the TCC grant attaches to the *sidecar's* code identity, not the app's. First `SCStream` start triggers the system prompt; denial surfaces as `CaptureError::PermissionDenied`.
- **Microphone** — `NSMicrophoneUsageDescription` must be present in the Info.plist of the process that opens the mic (the sidecar, or the app on its behalf), plus the TCC mic grant. Mic-source captures (`MicOnly`, `SystemAndMic`) gate on it; `SystemOnly` does not.
- **Surfacing** — the adapter maps a denied/undetermined grant to `CaptureError::PermissionDenied("screen recording" | "microphone")`; the app drives the user to System Settings. No silent failure.

### Ad-hoc-sign gotcha (risk — see Open Questions §3)

TCC binds a grant to the **code signature**. For the brew/ad-hoc path (no Developer ID), **re-signing on every rebuild changes the signature**, which can invalidate the existing grant and **re-prompt** for Screen Recording + Microphone. In dev (`swift build` / `cargo` rebuilds), this means a fresh prompt on most rebuilds. Compounding it: because the *sidecar* (a separate executable, not the `.app`) is what touches ScreenCaptureKit, TCC may track the **sidecar's own identity** rather than the parent app's — so "grant the app once" may not cover the helper unless the helper is signed as part of the bundle with a stable identity. This is the single biggest unknown in the slice. Mitigation candidates: a **stable self-signed dev identity** (sign the sidecar with a persistent local cert so the signature is stable across rebuilds), documenting the re-prompt as expected in dev, and deferring the packaged-bundle signing story to the packaging slice. Needs a maintainer call.

## Error handling

- Reuse `whisperkit_asr.rs`'s failure discipline: any stdio write/read error or EOF → `*slot = None` so the next call respawns; a sidecar that exits before acking → `CaptureError::Sidecar`; a truncated/over-cap line → a distinct framing error message.
- `Drop` on the adapter's session state closes stdin first (clean sidecar `readLine` EOF) then `kill` + `wait` to avoid a zombie on the error path — identical to `SidecarState::drop`.
- Poisoned mutexes map to typed `CaptureError`, never `panic!`, per the "no panics in production" rule.
- A capture that stops with zero kept screenshots is **valid** (conformance allows empty `screenshot_paths`); a zero-length/invalid WAV is **not** (the WAV must parse at 16 kHz mono).

## Testing

- **Unit (Rust, cheap):** `ScreenCaptureKitCapture` ↔ a **fake sidecar binary** (a stub that speaks the start/stop control protocol with canned acks, writes a tiny valid 16 kHz mono WAV + a fixture JPEG to the given paths) — passes the **same `Capture` conformance suite** as `FakeCapture`. The respawn-on-broken-pipe and `SessionNotFound` paths are exercised with a stub that drops the pipe / reports unknown session. No ScreenCaptureKit, no TCC, runs in CI.
- **Unit (Swift):** mix math (resample + sum-and-clamp for the three `audio_sources` modes) and dedup (FeaturePrint cosine threshold keeps/drops the right frames) on canned buffers/images.
- **Heavy / gated — human gate:** a **manual Mac smoke** on macOS 26 — real `SCStream`, grant TCC, run a short bilingual capture, confirm a playable mixed WAV + sensibly-deduped screenshots land in the meeting dir, and that the output feeds the existing ASR → diarization → notes pipeline. Gated exactly like the WhisperKit conformance smoke; **this is the maintainer's manual gate, not automatable in CI** (no screen/mic/TCC on the runner). Maps directly to north-star criterion #1.

## Three-tier boundaries

### ✅ Always do
- Run `cargo fmt --all && cargo build --workspace --locked && cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings` per task; build the Swift sidecar with `swift build --configuration release` + `swift test`.
- Keep `panops-core` platform-free; the adapter is `#[cfg(target_os="macos")]` in `panops-mac`.
- Mirror the `whisperkit_asr.rs` spawn/stdio/`Drop`/respawn machinery and the `asr_resolver` resolution order rather than inventing a new pattern.
- Surface every TCC denial as `CaptureError::PermissionDenied`; log sidecar detail to stderr, opaque code over the wire.
- Open a GitHub issue for each deferred item (live events, separate tracks, signing) per the Debt rule; commit per plan task.
- Verify pushed == local before relying on CI.

### ⚠️ Ask first
- Changing the `Capture` trait signature, `CaptureConfig`, or `CaptureResult` shape (e.g. adding separate-track paths, or injecting an event sink for the live path).
- Adding a `panops-protocol` dependency to `panops-mac` (would be needed for in-adapter live-event emission — couples the macOS adapter to wire types).
- Raising the SwiftPM minimum OS, or adding any Swift package dependency beyond Apple frameworks.
- Changing the screenshot dedup default (`0.15`) or interval (`500 ms`), or the mix strategy (sum-and-clamp vs average).
- Dropping or renaming a public type the engine/handlers already reference.

### 🚫 Never do
- Introduce a trait without one real impl + one fake. (The port already has both — do not add new ports speculatively, e.g. a separate `AudioCapture`/`VideoCapture` split — the trait doc explicitly forbids the pre-split.)
- Any network egress / telemetry from the sidecar or adapter (on-device only, zero telemetry ever).
- A user-facing env var for config (`PANOPS_CAPTURE_SIDECAR_BIN` is a dev/CI gate only, flagged here; no env var configures capture behavior).
- Persist or transmit captured audio/screenshots anywhere outside the meeting directory.
- Leave an `OnceLock` / once-init slot reachable in a permanent `None` state on an error or panic path.
- **Implement any capture code under this DRAFT** — this spec is doc-only until the maintainer approves it.
- Open or merge the PR autonomously (this PR is opened as a draft for the design gate; the maintainer merges).

## Acceptance criteria

1. On macOS 26 with the sidecar present and TCC granted, a `meeting.start` → short real capture → `meeting.stop` produces a single playable **16 kHz mono** WAV plus deduplicated screenshot JPEGs in `<meeting_dir>/`, and `CaptureResult` reports a positive `duration_ms`.
2. That output feeds the existing ASR → diarization → notes pipeline unchanged (north-star criterion #1; manual gate).
3. `panops-core` unchanged; `cargo test --workspace` green; `ScreenCaptureKitCapture` passes the same `Capture` conformance suite as `FakeCapture` via the fake sidecar.
4. A denied Screen Recording or Microphone grant surfaces as `CaptureError::PermissionDenied`, not a hang or panic.
5. No network egress from the sidecar or adapter; no new user-facing env vars.

## Risks

- **Ad-hoc-sign TCC re-prompt** (§3) — the dominant unknown; re-signing invalidates grants and the sidecar's separate identity may not inherit the app's grant. Mitigation in the permissions section; needs a maintainer decision.
- **Two-clock audio mixing** — system vs mic buffers arrive on independent clocks; gap-fill on presentation timestamps. Covered by the Swift mix unit tests; A/V sync is timestamp-tolerant for ASR.
- **CI can't exercise the real path** — no screen/mic/TCC on the runner, so the live capture is validated only by the manual Mac smoke. The fake-sidecar conformance test guards the adapter's framing/lifecycle, not ScreenCaptureKit itself.
- **`SCStream` frame-tap vs `SCScreenshotManager`** — frame-tap avoids a second stream but couples screenshot cadence to the video config; if it proves unreliable, fall back to `SCScreenshotManager` on-demand grabs (no contract change).
- **Resolver doc-path drift** — `capture_resolver.rs:16` references `2026-06-05-slice-11-live-capture-design.md`; this DRAFT is dated `2026-06-07`. Reconcile the comment when the spec is approved/renamed (trivial, non-blocking).

## Open questions (maintainer decisions before plan)

1. **Live event stream vs batch-at-stop for v0.1.** The wire types (`Screenshot`, `RecordingProgress`) exist but emitting them live requires the adapter to push events while the session runs — which means either injecting an event sink into the port (`Capture` change) or adding a `panops-protocol` dep to `panops-mac`. North-star scopes live streaming UI **out** of v0.1 ("live partials are bonus, not required"). **Recommendation: batch-at-stop for v0.1**, live events as a tracked follow-up. Confirm.
2. **Mixed vs separate audio tracks.** Mixing to one 16 kHz mono WAV matches the pipeline + conformance today. Keeping **separate** system/mic tracks is a near-free speaker split for diarization (system = remote participants, mic = local user) but changes `CaptureResult` to carry two paths (port change). **Recommendation: mixed for v0.1**, separate-tracks-for-diar as a follow-up enhancement. Confirm.
3. **Ad-hoc-sign TCC handling.** How should dev/brew handle the re-prompt — a stable self-signed dev identity to keep the signature constant across rebuilds, accept-and-document the re-prompt, or sign the sidecar as part of the bundle with a stable identity and defer the full story to packaging? Needs a maintainer call before the plan commits to a signing approach.
