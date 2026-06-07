# Slice 11 — Live Capture (Anchor B)

**Status:** **APPROVED 2026-06-07.** Design gate for #137. The three open questions are resolved (see *Decisions*). Implementation proceeds per `docs/superpowers/plans/2026-06-07-slice-11-live-capture.md`. Brainstorm: this file.

## Problem

North-star criterion #1 — *"Open the Mac app, hit record, run a real bilingual meeting → audio + screenshots captured"* — is unmet. Today the `Capture` port has a trait, a `FakeCapture`, a conformance suite, wire events, and handler wiring, but no real adapter: `capture_resolver::pick_capture()` returns `NotYetImplementedCapture` outside tests (`crates/panops-engine/src/capture_resolver.rs:46`). Live capture is **Anchor B** and the risk-last surface per the trajectory. This slice fills the gap.

## Goal

A macOS adapter that captures system audio + microphone + screenshots through a Swift ScreenCaptureKit sidecar, finalizes **two separate 16 kHz mono WAVs** (`system.wav` = remote participants, `mic.wav` = local user) plus deduplicated screenshot JPEGs into the meeting directory, and passes the (revised) `Capture` conformance suite. The two tracks let the pipeline split local-vs-remote speakers exactly — the mic track is the local user ("You"), the system track holds remote participants — sidestepping sherpa's tendency to over-count speakers in a 1:1 call. Completes **Anchor B**. This slice **does** change the `Capture` port contract (`CaptureResult` gains two track paths in place of one) — that is the one approved port change, scoped in *Decisions* below.

## What already exists (scaffolding, do not rebuild)

This slice is narrower than it looks — the Rust side is mostly wired. Already shipped:

- **Port** — `crates/panops-core/src/capture.rs`: `Capture` trait (`start_capture(meeting_id, meeting_dir, config) -> CaptureSession`, `stop_capture(session) -> CaptureResult`), `CaptureConfig { audio_sources, screenshot_interval_ms = 500, screenshot_threshold = 0.15 }`, `AudioSources { SystemOnly, MicOnly, SystemAndMic }`, `CaptureResult { audio_path, screenshot_paths, duration_ms }`, `CaptureError`. **This slice revises `CaptureResult`** — `audio_path` becomes `system_audio_path: Option<PathBuf>` + `mic_audio_path: Option<PathBuf>` (see *Decisions* §2 + *Port change*).
- **Fake** — `FakeCapture` (`conformance/fakes.rs:719`): synthetic 440 Hz sine WAV + fixture screenshots. **Revised this slice** to write two WAVs (one per present source) and populate the two new paths.
- **Conformance** — `conformance/capture.rs`: asserts a 16 kHz **mono** WAV, screenshot paths exist, `SessionNotFound` on unknown session, `is_fake` marker. **Revised this slice** to validate *each present track* is 16 kHz mono and to assert per-`AudioSources` track presence.
- **Resolver placeholder** — `capture_resolver.rs`: `OnceLock`-cached `pick_capture()`; `PANOPS_TEST_CAPTURE=1` → `FakeCapture`, else `NotYetImplementedCapture`.
- **Wire events** — `panops-protocol`: `Event::Screenshot(ScreenshotEvent { meeting_id, timestamp_ms, path })` and `Event::RecordingProgress(RecordingProgressEvent { meeting_id, bytes_captured, duration_ms })` (`methods.rs:80,89`). Defined; not yet emitted.
- **Handler wiring** — `server/handlers.rs`: `recording.start` / `recording.stop` (`handlers.rs:342,386`) already call `pick_capture()` → `start_capture` / `stop_capture`, with active sessions tracked in `Arc<Mutex<HashMap<String, CaptureSession>>>`. `RecordingStartParams` already carries `audio_sources` + screenshot knobs, mapped via `AudioSourcesWire`; `RecordingStopped` returns the captured paths.
- **Storage** — `MeetingStore::create_screenshot` / `list_screenshots` with `ScreenshotDraft { meeting_id, timestamp_ms, path, feature_print: Option<Vec<u8>>, caption }`.
- **Prod resolution helper** — `sidecar_binary::sibling_of_engine(name)` (current-exe sibling) + `executable_file` validation.

**The gap this slice closes:** (1) the two-track `CaptureResult` port change + conformance + fake, (2) the Swift `panops-capture-mac` sidecar (two WAVs + screenshots), (3) the `ScreenCaptureKitCapture` Rust adapter, (4) the resolver real tier, (5) the per-track local/remote attribution + timestamp-merge in the notes pipeline. Everything else is plumbing that already lands the result.

## Decisions (approved 2026-06-07)

The three open questions from the draft are resolved as follows. Everything downstream in this spec reflects these.

- **§1 — Batch-at-stop for v0.1 (live events deferred).** The sidecar accumulates audio + screenshots and returns the full `CaptureResult` at `stop_capture`. The live `Screenshot` / `RecordingProgress` wire events stay defined-but-unemitted until a follow-up wires them — north-star explicitly scopes live streaming UI out of v0.1 ("live partials are bonus, not required"). The line format for `screenshot` / `progress` notifications is reserved now so wiring them later is additive, not breaking.
- **§2 — Separate system/mic audio tracks (NOT mixed).** The sidecar writes **two** 16 kHz mono WAVs, not one mixed WAV: `system.wav` (remote participants, captured via SCStream `.audio`) and `mic.wav` (the local user, captured via `.microphone`). `CaptureResult` gains `system_audio_path: Option<PathBuf>` + `mic_audio_path: Option<PathBuf>`, **replacing** the single `audio_path`; each is populated only when its source is requested by `audio_sources`. This is the one approved **port change**. It buys an exact local-vs-remote speaker split for free (the mic track *is* the local user; the system track holds remote participants), which sidesteps sherpa over-counting speakers in a 1:1 call. `panops-core` stays platform-free; only the port struct + conformance + fake move.
- **§3 — Stable self-signed dev identity.** The capture sidecar is signed with a **constant local codesign identity** so the Screen-Recording (and Microphone) TCC grant survives rebuilds instead of re-prompting on every `swift build`. The identity is a one-time-created self-signed cert in the developer's login keychain; the `codesign` step is documented (see *Signing*) and run as part of the sidecar build. The packaged-bundle / Developer-ID / notarization signing story stays deferred to the packaging slice (#16).

These three carry into the design choices that were never in question:

- **Swift sidecar `apps/panops-capture-mac`**, mirroring `apps/panops-asr-mac` for layout, stdio framing, and the spawn/`Drop` lifecycle. ScreenCaptureKit + AVFoundation do the capture; Vision does screenshot dedup.
- **Separate buffers, separate output.** SCStream delivers system audio (`.audio`) and microphone (`.microphone`) as **separate** `CMSampleBuffer`s (macOS 26: `SCStreamConfiguration.capturesAudio` for system, `captureMicrophone` + `microphoneCaptureDeviceID` for mic). The sidecar resamples each to 16 kHz mono via `AVAudioConverter` and writes it to its **own** WAV — **no summing, no mixing**. Each track feeds the pipeline independently (see *Pipeline*).
- **`ScreenCaptureKitCapture` adapter** in `crates/panops-mac`, `impl Capture`, reusing the `whisperkit_asr.rs` spawn/stdio/`Drop` machinery — with one structural difference: capture is a **long-running session**, not request/response (see Architecture).
- **Resolver real tier** mirroring `asr_resolver::pick_asr`: `PANOPS_CAPTURE_SIDECAR_BIN` (dev/CI gate) → else `sibling_of_engine("panops-capture-mac")` (prod) → else the existing error path.

## Scope

### In

- **Port change** in `crates/panops-core`: `CaptureResult` gains `system_audio_path: Option<PathBuf>` + `mic_audio_path: Option<PathBuf>` in place of `audio_path`; `conformance/capture.rs` revised to validate each present track + per-`AudioSources` presence; `FakeCapture` revised to write two WAVs; downstream consumers (`RecordingStopped` wire type, `recording.stop` handler mapping) updated for compile-green. `panops-core` stays platform-free.
- `apps/panops-capture-mac/` SwiftPM executable (`platforms: [.macOS(.v26)]`):
  - `Sources/PanopsCaptureMac/main.swift` — stdio JSON-lines control loop (mirrors the ASR `main.swift` readLine pattern), routing `capture.start` / `capture.stop`.
  - `Recorder.swift` — owns the `SCStream`, `SCStreamOutput` for `.audio` / `.microphone` / `.screen`, AVFoundation per-source resample, **two** WAV writers (`system.wav`, `mic.wav`).
  - `Screenshotter.swift` — frame sampling at `screenshot_interval_ms`, Vision `VNGenerateImageFeaturePrint` cosine-distance dedup against the last kept frame at `screenshot_threshold`, JPEG write.
  - `Codecs.swift` — control-line request/response + event Codables.
  - Stable-identity `codesign` step in the build (see *Signing*).
- `crates/panops-mac/src/screencapturekit_capture.rs` — `ScreenCaptureKitCapture` impl `Capture`; lazy spawn, stdio control framing, `Drop` closes stdin + reaps child, respawn on broken pipe. Reuses the framing discipline from `whisperkit_asr.rs` (16 MiB line cap, `jsonrpc` version check, id pairing for the start/stop responses).
- `crates/panops-engine/src/capture_resolver.rs` — real tier (`PANOPS_CAPTURE_SIDECAR_BIN` → `sibling_of_engine` → error), `#[cfg(target_os="macos")]`, replacing `NotYetImplementedCapture` in prod while keeping the `PANOPS_TEST_CAPTURE` fake tier for tests.
- **Pipeline** in the notes path: per-track ASR, mic-track segments pinned to the local speaker (id 0, "You"), system-track segments diarized as remote (sherpa, ids offset after local), the two tracks merged by timestamp into one diarized `Transcript` (see *Pipeline*).
- Tests: `ScreenCaptureKitCapture` against a **fake sidecar binary** (a stub speaking the control protocol with canned responses) through the (revised) `Capture` conformance suite; the per-track attribution + merge logic unit-tested in `panops-core`; Swift unit tests for the resample + dedup math.

### Out (file as debt if surfaced)

- **Live event emission** (`Screenshot` / `RecordingProgress` over WebSocket) — deferred per §1; the wire types already exist, the emission path is a follow-up.
- **Production `Bundle.main` sidecar resolution + Developer-ID signing/entitlements/notarization** — lands in the packaging slice (#16); this slice keeps the `sibling_of_engine` + env gate and the *stable self-signed dev identity* (§3) for the local TCC grant only.
- **Screenshot captioning** (`ScreenshotRow.caption`) — already marked future-slice in the store.
- **Per-meeting `meeting.db` segment/screenshot persistence during live capture** — only if not already covered by the storage wiring; confirm against the current handler before filing.

## Architecture

```
panops-core (Capture port; CaptureResult gains system_audio_path + mic_audio_path)
        ▲
        │ impl
crates/panops-engine/src/capture_resolver.rs ── pick_capture() ──┐
        │  PANOPS_CAPTURE_SIDECAR_BIN (dev) | sibling (prod)      │ else
        ▼                                                         ▼
crates/panops-mac/ScreenCaptureKitCapture ──spawn/stdio control──►  NotYetImplementedCapture (error)
        │
        ▼
apps/panops-capture-mac (Swift, stable-signed):
    main ↔ Recorder(SCStream: .audio ─resample→ system.wav (16k mono)
         ↔                    .microphone ─resample→ mic.wav (16k mono))
         ↔ Screenshotter(SCStream frames → Vision FeaturePrint dedup → JPEG)

notes pipeline (per-track, see Pipeline):
    system.wav → VAD+ASR → sherpa diarize → remote turns (ids ≥ 1)  ┐
    mic.wav    → VAD+ASR → all segments → local speaker (id 0 "You") ┤→ merge by ts → Transcript
```

### The structural difference from ASR/LLM sidecars

WhisperKit and FoundationModels sidecars are **request/response**: one `transcribe` / `complete` line in, one result line out, id-paired. Capture is a **stateful session**:

1. `start_capture` spawns the sidecar (or reuses a live one), writes one `capture.start` control line `{ id, meeting_id, system_audio_path, mic_audio_path, screenshots_dir, audio_sources, screenshot_interval_ms, screenshot_threshold }`, and reads one `capture.started` ack. The sidecar then runs `SCStream` in the background, writing each requested source to its own WAV and screenshots to disk. **No further lines flow until stop** (batch-at-stop). The engine derives the two WAV paths from `meeting_dir` (`<meeting_dir>/system.wav`, `<meeting_dir>/mic.wav`) and passes only the ones implied by `audio_sources`.
2. `stop_capture` writes one `capture.stop` control line `{ id, meeting_id }`; the sidecar stops the stream, finalizes each open WAV, and returns `{ system_audio_path, mic_audio_path, screenshot_paths, duration_ms }` (each audio path null when its source was not requested) as the response. The adapter maps that to `CaptureResult`.

So the adapter reuses the **spawn/stdio/`Drop`/respawn machinery** from `whisperkit_asr.rs` verbatim in shape, but the control protocol is start/ack + stop/result rather than a single paired call. The 16 MiB line cap, `deserialize_jsonrpc_version`, id-pairing, and `*slot = None` on stdio failure all carry over. Per AGENTS.md, any `OnceLock` / once-init slot the resolver exposes must reach a terminal state on every path (success, error, panic) — wrap heavy init in `catch_unwind` as precedent `02559a3` did.

## Control protocol (JSON-lines over stdio, mirrors ASR framing)

- `capture.start` params `{ meeting_id, system_audio_path: string|null, mic_audio_path: string|null, screenshots_dir, audio_sources: "system_only"|"mic_only"|"system_and_mic", screenshot_interval_ms, screenshot_threshold }` → result `{ started_at_ms }` or error → `CaptureError::{PermissionDenied, Sidecar, InvalidConfig}`. A null path means "do not capture that source"; the pair MUST agree with `audio_sources` (the adapter sends only the requested paths).
- `capture.stop` params `{ meeting_id }` → result `{ system_audio_path: string|null, mic_audio_path: string|null, screenshot_paths: [..], duration_ms }` → `CaptureResult`. Each audio path is non-null exactly when that source was captured. Unknown session → `CaptureError::SessionNotFound`.
- Errors map to `CaptureError` at the adapter boundary. Full sidecar detail (ScreenCaptureKit / TCC state) goes to **stderr** (Console.app); an opaque code-only message goes over the wire, matching the ASR sidecar's leak discipline.
- **(deferred, §1)** `screenshot` / `progress` JSON-RPC *notifications* (no id) for the live path — the line format is reserved now so wiring them later is additive, not breaking.

## Port change (`CaptureResult`)

This is the one approved change to a locked port. It is intentionally minimal — only `CaptureResult` moves; the trait signatures, `CaptureConfig`, `CaptureSession`, and `CaptureError` are untouched.

**Before:**

```rust
pub struct CaptureResult {
    pub audio_path: PathBuf,
    pub screenshot_paths: Vec<PathBuf>,
    pub duration_ms: u64,
}
```

**After:**

```rust
pub struct CaptureResult {
    /// System-audio (remote participants) track. `None` when `audio_sources`
    /// did not request system audio (i.e. `MicOnly`).
    pub system_audio_path: Option<PathBuf>,
    /// Microphone (local user) track. `None` when `audio_sources` did not
    /// request the mic (i.e. `SystemOnly`).
    pub mic_audio_path: Option<PathBuf>,
    pub screenshot_paths: Vec<PathBuf>,
    pub duration_ms: u64,
}
```

Invariant tying the result to config: for `SystemOnly`, `mic_audio_path == None` and `system_audio_path == Some`; for `MicOnly`, the reverse; for `SystemAndMic`, both `Some`. At least one is always `Some` for a successful capture.

**Blast radius (all updated in the same task for compile-green):**

- `crates/panops-core/src/capture.rs` — the struct + its unit tests (`capture_result_paths`).
- `crates/panops-core/src/conformance/capture.rs` — `stop_returns_valid_audio` validates *each present track* (16 kHz mono via `hound`) and asserts the per-`AudioSources` presence invariant; `stop_returns_screenshot_paths` unchanged.
- `crates/panops-core/src/conformance/fakes.rs` — `FakeCapture::stop_capture` writes `system.wav` and/or `mic.wav` per the session's `audio_sources` (it must record `audio_sources` at `start_capture`) and populates the two new fields.
- `crates/panops-protocol/src/methods.rs` — `RecordingStopped` gains `system_audio_path: Option<String>` + `mic_audio_path: Option<String>` in place of `audio_path` (round-trip test updated).
- `crates/panops-engine/src/server/handlers.rs` — `recording_stop` maps the two `Option<PathBuf>` to the two wire fields (`handlers.rs:404-414`).

The notes pipeline's consumption of the two paths is a separate task (see *Pipeline*); the port-change task only needs the workspace to compile and existing tests (plus the revised conformance) to pass.

## Audio tracks (two WAVs, no mix)

Per **Decision §2**, the sidecar writes **two** independent 16 kHz mono PCM WAVs — never a mixed one. Each present track is still a valid 16 kHz mono WAV (conformance asserts `sample_rate == 16_000`, `channels == 1` *for each present track*). macOS 26 delivers system and mic audio as **separate** sample-buffer streams, so the sidecar:

1. Receives `.audio` (system, typically 48 kHz stereo) and `.microphone` (device-native) buffers on the `SCStreamOutput` callback.
2. Resamples each to 16 kHz mono via `AVAudioConverter` (stereo → mono downmix is an average of L+R *within one source* — this is channel downmix, not cross-source mixing).
3. Routes each per `audio_sources`: `SystemOnly` opens only `system.wav`; `MicOnly` opens only `mic.wav`; `SystemAndMic` opens both. Each resampled buffer is appended to **its own** WAV (16-bit PCM) via the same `hound`-equivalent writer path. **No summing across sources.**
4. On `capture.stop`, finalizes each open WAV and reports its path (the un-opened source's path is null).

Both WAVs share the **same capture-start clock**: each buffer is written at its presentation timestamp relative to capture start, with zero-fill for gaps, so a timestamp `t` on the mic track and `t` on the system track refer to the same wall-clock instant. That shared clock is what makes the cross-track timestamp merge in *Pipeline* sound. Absolute A/V sync to the screenshots is timestamp-tolerant (ASR is tolerant within a segment).

## Screenshot sampling, dedup, time-anchoring

- **Sampling** — every `screenshot_interval_ms` (default 500), grab the latest `SCStream` video frame (or `SCScreenshotManager` for an on-demand grab; frame-tap preferred to avoid a second stream).
- **Dedup** — compute a Vision `VNGenerateImageFeaturePrintRequest` feature print, cosine-distance it against the last **kept** frame's print. Below `screenshot_threshold` (default 0.15) → drop as a near-duplicate; at/above → keep, write JPEG, update the reference print. This is the same FeaturePrint dedup the `CaptureConfig` knobs were designed around.
- **Time-anchoring** — each kept JPEG is named/recorded with `timestamp_ms` relative to capture start (the same clock as audio sample offsets), so notes generation can anchor a screenshot to the transcript segment active at that moment. The feature print is retained for the eventual `ScreenshotRow.feature_print` column.

## Pipeline (per-track attribution + timestamp merge)

The two tracks are what make local-vs-remote attribution exact. Today the notes path (`handlers.rs:562-630`) loads **one** audio file, runs VAD → recursive ASR → stitched segments, then optionally diarizes the whole file with sherpa and overlays speaker ids via `merge_speaker_turns` (`merge.rs:7`). Sherpa clusters by voice embedding, and on a 1:1 call it routinely over- or under-counts (the kind of spurious cluster `636efbd` had to collapse). With two tracks we never ask sherpa to separate local from remote — the **track itself** tells us — so sherpa only runs on the system track to split *multiple remote* speakers.

**Speaker-id convention:**

- `id 0` = **local user**, label **"You"**. Reserved whether or not the mic was captured.
- `id ≥ 1` = **remote participants**, one per sherpa cluster on the system track, mapped `remote_id = sherpa_id + 1` (offset past the reserved local id).

**Algorithm (a new function, e.g. `panops_core::pipeline::transcribe_two_track`, called from `notes.generate` when two paths are present):**

1. **Mic track** (`mic_audio_path`, if `Some`): run the existing VAD + recursive-ASR path (`load_audio_mono16k` → `vad.detect_speech` → `merge_adjacent_regions` → `transcribe_recursive`) to get segments with absolute timestamps. Pin **every** segment to `speaker_id = Some(0)`. No diarization — it is one known local speaker.
2. **System track** (`system_audio_path`, if `Some`): run the same VAD + recursive-ASR path to get segments. Then `diar.diarize(system.wav)` → `Vec<SpeakerTurn>`; offset each turn's `speaker_id` by `+1`; assign to the system segments with `merge_speaker_turns` (segments with no overlapping turn keep `None`, exactly as today).
3. **Merge** the two segment lists into one `Vec<Segment>` ordered by `start_ms` (stable sort; on equal `start_ms`, mic before system is fine — overlap is rare and the downstream notes IR tolerates it). Build the final `Transcript { diarized: true, segments, audio_duration_ms = max(track durations), .. }`.
4. **Degenerate cases:** only one track present → run just that branch (mic-only → all id 0, no sherpa; system-only → sherpa with the `+1` offset, no id-0 segments). Both `None` is impossible for a successful `CaptureResult`.

This sidesteps the sherpa 1:1 over-count because the dominant split (you vs. them) is decided by track origin, not clustering; sherpa's only job is the easier sub-problem of separating remote speakers from each other on a track that excludes the local mic.

**Wiring note:** `notes.generate` currently takes a single `audio` param (`handlers.rs:521`). The two-track entry is reached when the meeting was produced by live capture (two WAVs on disk). How the handler selects single-track vs two-track (e.g. a new optional `system_audio`/`mic_audio` param pair on `NotesGenerateParams`, or detecting `system.wav`/`mic.wav` in the meeting dir) is a task-level decision in the plan; the single-track legacy path (file-import meetings) stays unchanged. The label map (`0 → "You"`) is applied where speaker ids are rendered (notes IR / exporter), consistent with how speaker ids already surface.

## Permissions / TCC flow

- **Screen Recording** (TCC) — required for `SCStream`. The **sidecar process** runs ScreenCaptureKit, so the TCC grant attaches to the *sidecar's* code identity, not the app's. First `SCStream` start triggers the system prompt; denial surfaces as `CaptureError::PermissionDenied`.
- **Microphone** — `NSMicrophoneUsageDescription` must be present in the Info.plist of the process that opens the mic (the sidecar, or the app on its behalf), plus the TCC mic grant. Mic-source captures (`MicOnly`, `SystemAndMic`) gate on it; `SystemOnly` does not.
- **Surfacing** — the adapter maps a denied/undetermined grant to `CaptureError::PermissionDenied("screen recording" | "microphone")`; the app drives the user to System Settings. No silent failure.

## Signing — stable self-signed dev identity (Decision §3)

**The problem.** TCC binds a grant to the **code signature**. ScreenCaptureKit runs inside the *sidecar* (a separate executable, not the `.app`), so TCC tracks the **sidecar's own identity**. An ad-hoc / re-signed-every-build binary changes signature on most rebuilds, which invalidates the Screen-Recording + Microphone grants and **re-prompts** the developer constantly. That makes the manual Mac smoke (the only test of the real path) miserable to run.

**The decision.** Sign `panops-capture-mac` with a **constant local self-signed identity** so its signature is stable across rebuilds and the TCC grant persists. The identity is a one-time, machine-local self-signed code-signing certificate in the developer's login keychain — **not** a Developer-ID cert, **not** committed, **not** required to *build* (only to get a persistent dev grant).

**One-time setup (documented in the sidecar README / build docs, not automated):** create a self-signed code-signing certificate named e.g. `panops-dev` via Keychain Access → *Certificate Assistant → Create a Certificate* (type: Code Signing), or the `security` CLI equivalent. This is per-developer-machine and run once.

**Per-build step (run by the sidecar build task):** after `swift build`, sign the product with the stable identity:

```bash
codesign --force --options runtime \
  --sign "panops-dev" \
  .build/release/panops-capture-mac
```

If the `panops-dev` identity is absent (CI, a fresh checkout, or a contributor who hasn't created it), the build **falls back to ad-hoc** (`--sign -` or no signing) and the slice still builds + the fake-sidecar conformance test still passes — only the persistent-grant convenience is lost, and the manual smoke re-prompts. So the stable identity is a dev-ergonomics optimization, never a hard build dependency.

**Scope boundary.** Microphone also needs `NSMicrophoneUsageDescription` in the signed binary's embedded `Info.plist`; the sidecar's `Package.swift` / build embeds a minimal plist with that key (and a usage string). The **packaged-bundle** signing story — Developer-ID, hardened-runtime entitlements, notarization, embedding the sidecar in the `.app` so it inherits the app's identity — stays deferred to the **packaging slice (#16)**. This slice only solves the local-dev TCC-persistence problem.

## Error handling

- Reuse `whisperkit_asr.rs`'s failure discipline: any stdio write/read error or EOF → `*slot = None` so the next call respawns; a sidecar that exits before acking → `CaptureError::Sidecar`; a truncated/over-cap line → a distinct framing error message.
- `Drop` on the adapter's session state closes stdin first (clean sidecar `readLine` EOF) then `kill` + `wait` to avoid a zombie on the error path — identical to `SidecarState::drop`.
- Poisoned mutexes map to typed `CaptureError`, never `panic!`, per the "no panics in production" rule.
- A capture that stops with zero kept screenshots is **valid** (conformance allows empty `screenshot_paths`); a zero-length/invalid WAV on a **present** track is **not** (each non-null track path must parse at 16 kHz mono). A null track path is valid only when `audio_sources` did not request that source.

## Testing

- **Unit (Rust, cheap):** `ScreenCaptureKitCapture` ↔ a **fake sidecar binary** (a stub that speaks the start/stop control protocol with canned acks, writes a tiny valid 16 kHz mono WAV per requested source + a fixture JPEG to the given paths) — passes the (revised) `Capture` conformance suite, including the per-`AudioSources` track-presence invariant. The respawn-on-broken-pipe and `SessionNotFound` paths are exercised with a stub that drops the pipe / reports unknown session. No ScreenCaptureKit, no TCC, runs in CI.
- **Unit (Rust, cheap):** `transcribe_two_track` attribution + merge — with a fake ASR + fake diarizer, assert mic segments get `speaker_id 0`, system segments get ids `≥ 1` (offset), the merged list is timestamp-ordered, and each degenerate single-track mode behaves (mic-only → all id 0; system-only → no id 0).
- **Unit (Swift):** per-source resample/route (each `audio_sources` mode opens the right WAV set, no cross-source summing) and dedup (FeaturePrint cosine threshold keeps/drops the right frames) on canned buffers/images.
- **Heavy / gated — human gate:** a **manual Mac smoke** on macOS 26 — real `SCStream`, grant TCC, run a short bilingual capture, confirm **two** playable 16 kHz mono WAVs (`system.wav`, `mic.wav`) + sensibly-deduped screenshots land in the meeting dir, and that the output feeds the two-track ASR → diarization → notes pipeline with a correct local-vs-remote split. Gated exactly like the WhisperKit conformance smoke; **this is the maintainer's manual gate, not automatable in CI** (no screen/mic/TCC on the runner). Maps directly to north-star criterion #1.

## Three-tier boundaries

### ✅ Always do
- Run `cargo fmt --all && cargo build --workspace --locked && cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings` per task; build the Swift sidecar with `swift build --configuration release` + `swift test`.
- Keep `panops-core` platform-free; the adapter is `#[cfg(target_os="macos")]` in `panops-mac`.
- Mirror the `whisperkit_asr.rs` spawn/stdio/`Drop`/respawn machinery and the `asr_resolver` resolution order rather than inventing a new pattern.
- Surface every TCC denial as `CaptureError::PermissionDenied`; log sidecar detail to stderr, opaque code over the wire.
- Apply the approved two-track `CaptureResult` change (Decision §2) as scoped in *Port change* — this is the **one** sanctioned port change; everything else about the trait stays fixed.
- Sign the sidecar with the stable `panops-dev` identity when present, ad-hoc otherwise (Decision §3); never make the stable identity a hard build dependency.
- Open a GitHub issue for each deferred item (live events, packaging-bundle signing) per the Debt rule; commit per plan task.
- Verify pushed == local before relying on CI.

### ⚠️ Ask first
- Any **further** change to the `Capture` trait signature, `CaptureConfig`, `CaptureSession`, or `CaptureResult` **beyond** the approved two-path swap (e.g. injecting an event sink for the live path, or adding a third audio path).
- Adding a `panops-protocol` dependency to `panops-mac` (would be needed for in-adapter live-event emission — couples the macOS adapter to wire types).
- Raising the SwiftPM minimum OS, or adding any Swift package dependency beyond Apple frameworks.
- Changing the screenshot dedup default (`0.15`) or interval (`500 ms`), or the speaker-id convention (`0` = local "You", remote `≥ 1`).
- Dropping or renaming a public type the engine/handlers already reference (beyond the approved `audio_path` → two-path swap and its `RecordingStopped` mirror).

### 🚫 Never do
- Introduce a trait without one real impl + one fake. (The port already has both — do not add new ports speculatively, e.g. a separate `AudioCapture`/`VideoCapture` split — the trait doc explicitly forbids the pre-split.)
- Any network egress / telemetry from the sidecar or adapter (on-device only, zero telemetry ever).
- A user-facing env var for config (`PANOPS_CAPTURE_SIDECAR_BIN` is a dev/CI gate only, flagged here; no env var configures capture behavior).
- Persist or transmit captured audio/screenshots anywhere outside the meeting directory.
- Leave an `OnceLock` / once-init slot reachable in a permanent `None` state on an error or panic path.
- **Mix the two audio sources into one WAV** — the tracks stay separate (Decision §2); summing across sources is what we explicitly avoided.
- Open or merge the slice PR autonomously — the maintainer opens and merges; agents drive the plan to a pushed branch.

## Acceptance criteria

1. On macOS 26 with the sidecar present and TCC granted, a `recording.start` (`SystemAndMic`) → short real capture → `recording.stop` produces **two** playable **16 kHz mono** WAVs (`system.wav`, `mic.wav`) plus deduplicated screenshot JPEGs in `<meeting_dir>/`, and `CaptureResult` reports a positive `duration_ms` with both audio paths `Some`. `SystemOnly` / `MicOnly` produce exactly the requested track and `None` for the other.
2. That output feeds the **two-track** ASR → diarization → notes pipeline, yielding a transcript where the local user is `speaker_id 0` ("You") and remote participants are `≥ 1` (north-star criterion #1; manual gate).
3. `panops-core` stays platform-free (the only change is the `CaptureResult` struct + its conformance/fake); `cargo test --workspace` green; `ScreenCaptureKitCapture` passes the revised `Capture` conformance suite as `FakeCapture` does, via the fake sidecar.
4. A denied Screen Recording or Microphone grant surfaces as `CaptureError::PermissionDenied`, not a hang or panic.
5. No network egress from the sidecar or adapter; no new user-facing env vars; the two tracks live only under `<meeting_dir>/`.

## Risks

- **Two-clock track alignment** — system vs mic buffers arrive on independent clocks; both are written on presentation timestamps relative to capture start with zero-fill, so the cross-track timestamp merge stays sound. Covered by the Swift resample/route unit tests + the Rust merge tests; A/V sync to screenshots is timestamp-tolerant for ASR. (Lower risk than mixing — no sample-accurate sum is required, only per-track timestamps.)
- **Stable-identity drift** — if a developer's `panops-dev` cert is missing or regenerated, the TCC grant re-prompts; the build falls back to ad-hoc so nothing *breaks*, but the smoke loses grant persistence. Documented in *Signing*; non-blocking.
- **CI can't exercise the real path** — no screen/mic/TCC on the runner, so the live capture is validated only by the manual Mac smoke. The fake-sidecar conformance test guards the adapter's framing/lifecycle, not ScreenCaptureKit itself.
- **`SCStream` frame-tap vs `SCScreenshotManager`** — frame-tap avoids a second stream but couples screenshot cadence to the video config; if it proves unreliable, fall back to `SCScreenshotManager` on-demand grabs (no contract change).
- **Sherpa on the system track still over-counts** — separating remote speakers from each other is the easier sub-problem (the local mic is excluded), but a noisy system track can still over-count remote participants. That's a quality issue inside the `≥ 1` id space, not a local/remote-split failure; the existing spurious-cluster collapse (`636efbd`) still applies. File as calibration debt if the smoke surfaces it.
- **Resolver doc-path drift** — `capture_resolver.rs:16` references `2026-06-05-slice-11-live-capture-design.md`; this spec is dated `2026-06-07`. **Reconcile the comment in the T4 resolver task** (trivial, non-blocking) — noted here so it isn't lost.

## Resolved questions

The three open questions are decided (see *Decisions, approved 2026-06-07*): §1 batch-at-stop (live events deferred), §2 separate system/mic tracks (the approved port change), §3 stable self-signed dev identity. Nothing remains gating the plan.
