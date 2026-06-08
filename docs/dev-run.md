# Running Panops in development

How to build and launch the Mac app from source during development. (For the
distributable, brew-installable `.app`, see `docs/release-v0.1.md`.)

## TL;DR — two commands

```bash
# A) Quick look (UI iteration). Builds engine + capture sidecar + runs the app
#    from source. Command Line Tools is enough. Live Screen Recording will NOT
#    work here (a bare swift-run binary can't get the macOS Screen Recording
#    permission) — use this to browse the UI / generate notes from an audio file.
scripts/dev.sh

# B) Full flow WITH permissions (record → transcribe → notes). Builds + ad-hoc
#    signs Panops.app (all three sidecars bundled), so macOS will grant Screen
#    Recording + Microphone. Requires FULL Xcode (ASR/LLM sidecars) + macOS 26.
scripts/package.sh
open dist/Panops.app
#    Then System Settings → Privacy & Security → grant Screen Recording + Mic,
#    relaunch the app, and record a meeting.
```

Details + the manual smoke checklist below.

## Concept: there is no `.app` icon during dev

The Mac app lives at `apps/Panops/` as a **SwiftPM executable** (source), not a
double-clickable `Panops.app` bundle. In development you launch it from the
terminal with `swift run`. The clickable `Panops.app` bundle is produced later
by `scripts/package.sh` (the distribution artifact). So "where is the app" = you
build and run it.

The app **spawns its own `panops-engine`** on launch (over a Unix socket at
`~/Library/Application Support/panops/engine.sock`) and resolves the engine
binary from the `PANOPS_ENGINE_BIN` env var (dev escape hatch).

## Level 1 — see the app (a few minutes)

```bash
# 1. build the engine the app will spawn
cargo build -p panops-engine

# 2. build + launch the app (a window opens)
cd apps/Panops
PANOPS_ENGINE_BIN="$(git rev-parse --show-toplevel)/target/debug/panops-engine" swift run
```

A window opens: the meeting-list sidebar, the detail pane (transcript / notes /
screenshots), and the **New Recording** button. On a healthy launch the engine
log prints `heavy adapters ready` and the socket above appears. Without
`PANOPS_ENGINE_BIN` the window still opens but shows "engine not connected /
Retry".

This is enough to browse meetings, generate notes from an existing audio file,
and click through the UI.

## Level 2 — full record → transcribe → notes (the manual smoke)

Two extra prerequisites the dev `swift run` path does not cover:

### 1. Full Xcode (for the ASR + LLM sidecars)

The WhisperKit ASR sidecar (`apps/panops-asr-mac`, CoreML) and the
FoundationModels LLM sidecar (`apps/panops-llm-mac`) need **full Xcode**, not
just the Command Line Tools. Check and switch:

```bash
xcode-select -p   # if this prints .../CommandLineTools, install Xcode first
sudo xcode-select -s /Applications/Xcode.app
```

Build the sidecars (release) and point the engine at them:

```bash
( cd apps/panops-asr-mac && swift build -c release )
( cd apps/panops-llm-mac && swift build -c release )
( cd apps/panops-capture-mac && swift build -c release )

export PANOPS_ASR_SIDECAR_BIN="$(git rev-parse --show-toplevel)/apps/panops-asr-mac/.build/release/panops-asr-mac"
export PANOPS_LLM_SIDECAR_BIN="$(git rev-parse --show-toplevel)/apps/panops-llm-mac/.build/release/panops-llm-mac"
export PANOPS_CAPTURE_SIDECAR_BIN="$(git rev-parse --show-toplevel)/apps/panops-capture-mac/.build/release/panops-capture-mac"
```

The LLM sidecar also needs **Apple Intelligence enabled** (System Settings →
Apple Intelligence & Siri) or notes generation returns empty sections.

### 2. A signed `.app` bundle (for Screen Recording TCC)

macOS grants the **Screen Recording** permission only to a proper signed app
bundle, not a bare `swift run` binary. For real capture, build and open the
bundle:

```bash
scripts/package.sh v0.1.0      # → dist/Panops.app (+ tar.gz + sha256)
open dist/Panops.app
```

Then grant **Screen Recording** and **Microphone** in
System Settings → Privacy & Security. (To keep the TCC grant stable across
rebuilds, create the local `panops-dev` signing identity — see
`apps/panops-capture-mac/README.md`.)

Now: open the app → **New Recording** → run a real meeting → Stop → the
two-track transcript appears → **Generate notes** → markdown with screenshots
and action items.

## Tests

```bash
cargo test --workspace --locked          # Rust
( cd apps/Panops && swift test )         # Mac app (needs the CLT framework
                                         # search paths on a CLT-only machine —
                                         # see reference in the repo)
```
