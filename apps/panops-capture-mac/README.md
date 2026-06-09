# panops-capture-mac

ScreenCaptureKit live-capture sidecar for the panops engine. Slice 11 (Anchor B).
Spec: `docs/superpowers/specs/2026-06-07-slice-11-live-capture-design.md`.

Captures **system audio**, **microphone**, and **screen frames** through one
`SCStream`, writing **two separate 16 kHz mono WAVs** (`system.wav` = all system
audio output except this sidecar's own — during a call that is the remote
participants, but it is whole-system audio, not per-app isolation; `mic.wav` =
the local user's microphone — never mixed) plus deduplicated screenshot JPEGs
into the meeting directory. Everything stays on this Mac; the sidecar performs
zero network egress.

## Architecture

```
panops-engine (Rust)
    ↓ spawn + JSON-RPC over stdio (stateful start/ack + stop/result)
panops-capture-mac (Swift)
    ├─ Recorder       SCStream .audio  ─AVAudioConverter→ 16k mono → system.wav
    │                 SCStream .microphone ──────────────→ 16k mono → mic.wav
    └─ Screenshotter  SCStream .screen → Vision FeaturePrint dedup → JPEGs
```

Unlike the request/response ASR sidecar, capture is a **session**:
`capture.start` opens the stream and acks; `capture.stop` finalizes the WAVs and
returns the paths (batch-at-stop — live `screenshot`/`progress` notifications are
reserved but unemitted for v0.1).

## Control protocol (JSON-lines over stdio)

`capture.start`:

```json
{"jsonrpc":"2.0","id":1,"method":"capture.start","params":[{
  "meeting_id":"<uuid>",
  "system_audio_path":"<meeting_dir>/system.wav",
  "mic_audio_path":"<meeting_dir>/mic.wav",
  "screenshots_dir":"<meeting_dir>/screenshots",
  "audio_sources":"system_and_mic",
  "screenshot_interval_ms":500,
  "screenshot_threshold":0.15,
  "capture_target":{"kind":"display"}
}]}
```

→ `{"jsonrpc":"2.0","id":1,"result":{"started_at_ms":1700000000000}}`

A `null` audio path means "do not capture that source"; the paths MUST agree
with `audio_sources` (`system_only` / `mic_only` / `system_and_mic`).

`capture_target` selects what the `SCStream` captures: `{"kind":"display"}`
(default; omit the field for the same effect) captures the first display, and
`{"kind":"window","window_id":<u32>}` captures a single window by its
`SCWindow.windowID`. The app's `SCContentSharingPicker` resolves the window id;
an unknown `window_id` falls back to full-display capture (logged to stderr);
capture never fails on it.

`capture.stop`:

```json
{"jsonrpc":"2.0","id":2,"method":"capture.stop","params":[{"meeting_id":"<uuid>"}]}
```

→ `{"jsonrpc":"2.0","id":2,"result":{"system_audio_path":"...","mic_audio_path":"...","screenshot_paths":["..."],"duration_ms":12345}}`

Each audio path is non-null exactly when its source was captured. Unknown
`meeting_id` → `error.code -32004` ("session not found"). Screen-Recording
denial → `-32001`; Microphone denial → `-32002`. Full ScreenCaptureKit / TCC
detail goes to **stderr** (Console.app); only an opaque code reaches the wire.

## Build

```bash
cd apps/panops-capture-mac
swift build --configuration release
swift test
```

Then point the engine at the binary (dev/CI gate only — not a user-facing
config var):

```bash
export PANOPS_CAPTURE_SIDECAR_BIN="$PWD/.build/release/panops-capture-mac"
```

## Signing — stable self-signed dev identity

TCC binds the Screen-Recording and Microphone grants to the sidecar's **code
signature**. An ad-hoc binary changes signature on most rebuilds, which
invalidates the grants and re-prompts on every `swift build`. To keep the grant
across rebuilds, sign with a **constant local self-signed identity**.

### One-time setup (per developer machine)

Create a self-signed **Code Signing** certificate named `panops-dev`:

- Keychain Access → *Certificate Assistant* → *Create a Certificate…*
  - Name: `panops-dev`
  - Identity Type: *Self-Signed Root*
  - Certificate Type: *Code Signing*

(or the `security`/`certtool` CLI equivalent). This cert is machine-local, is
**not** committed, and is **not** a Developer-ID cert.

### Per-build sign

After building, sign the product with the stable identity, falling back to
ad-hoc when `panops-dev` is absent (CI, fresh checkout, a contributor who hasn't
created it):

```bash
swift build --configuration release
codesign --force --options runtime --sign "panops-dev" \
  .build/release/panops-capture-mac \
  || codesign --force --sign - .build/release/panops-capture-mac   # ad-hoc fallback
```

The stable identity is a **dev-ergonomics optimization only**: it is not
required to build, and the fake-sidecar conformance test (Rust, CI) never needs
it. When the identity is missing the build still works — only the persistent
TCC grant is lost and the manual smoke re-prompts.

The embedded `Info.plist` (`Resources/Info.plist`, linked into
`__TEXT,__info_plist`) carries `NSMicrophoneUsageDescription` +
`NSScreenCaptureUsageDescription` so the TCC prompts show a usage string.

> **Deferred to the packaging slice (#16):** the packaged-bundle signing story —
> Developer-ID, hardened-runtime entitlements, notarization, and embedding the
> sidecar in the `.app` so it inherits the app's identity. This slice solves
> only the local-dev TCC-persistence problem.

## Requirements

- macOS 26.0+
- Swift 6 toolchain (Command Line Tools are enough — no Xcode CoreML toolchain
  needed, unlike the ASR sidecar).
- TCC grants: **Screen Recording** (always) and **Microphone** (for `mic_only` /
  `system_and_mic`). First `SCStream` start triggers the system prompt.

## Testing

`swift test` covers the pure logic that runs without a display/mic/TCC:

- **Audio routing** — each `audio_sources` mode opens the right WAV set.
- **Resample quantization** — Float32 → Int16 clamping/rounding.
- **WAV header** — 16 kHz / mono / 16-bit framing + round-trip.
- **Screenshot dedup** — cosine-distance keep/drop vs the `0.15` threshold.

The live ScreenCaptureKit → two-WAV + screenshot path is validated by the
**manual Mac smoke** (maintainer gate) — CI has no screen/mic/TCC.

Tests use **Swift Testing** (`import Testing`), not XCTest, because XCTest ships
only with full Xcode while Swift Testing is bundled with the toolchain. With
full Xcode selected, `swift test` works as-is. On a **Command Line Tools-only**
machine the Testing framework needs its search paths passed explicitly:

```bash
FW=/Library/Developer/CommandLineTools/Library/Developer/Frameworks
LIB=/Library/Developer/CommandLineTools/Library/Developer/usr/lib
swift test \
  -Xswiftc -F -Xswiftc "$FW" \
  -Xlinker -rpath -Xlinker "$FW" \
  -Xlinker -rpath -Xlinker "$LIB"
```
