# Slice 16 — Packaging: brew-installable, ad-hoc-signed `.app` (v0.1 criterion #6)

**Status:** design approved 2026-06-06 (maintainer). Brainstorm: this file. Plan: forthcoming via `superpowers:writing-plans`.

## Problem

v0.1 criterion #6 wants the product runnable on a **clean Mac with no dev tools**. Today `swift build` produces a CLI binary, not a `.app`; nothing assembles a bundle, signs it, or distributes it. The engine only finds the sidecars through the dev/CI env gates (`PANOPS_ASR_SIDECAR_BIN` / `PANOPS_LLM_SIDECAR_BIN`) — in a shipped bundle there's no such env, so it would fall back to whisper-rs / Ollama instead of the WhisperKit + FoundationModels sidecars. Models download from upstream HuggingFace/GitHub with no project-controlled mirror (#8). And the maintainer wants to ship **without paying the $99/yr Apple Developer fee** for v0.1 — proving the app first, paying later.

## Goal

A `vfmatzkin/homebrew-panops` tap that installs an **ad-hoc-signed `Panops.app`** bundling the engine + both sidecars, where the engine **self-locates the sidecars at runtime** (no env var), and models **download on first run from a project-controlled mirror**. No Apple Developer fee for v0.1; the paid notarized channel is a drop-in addition later.

## Decisions (locked)

- **Brew distribution, no Apple fee.** A custom Homebrew **cask** in `vfmatzkin/homebrew-panops` (standard `.app` → `/Applications`). Ad-hoc signed (`codesign -s -`, free). Un-notarized → the cask `caveats` documents the one-time Gatekeeper override. Verified rationale: Apple Silicon requires *ad-hoc* signing (free) to run; Gatekeeper only blocks *quarantined* apps; third-party taps are exempt from Homebrew 5.0's notarization requirement (which hits only the official cask repo, Sept 2026).
- **Notarization-ready.** Sign with **hardened runtime + entitlements** now, so the later paid path is just swapping `-s -` for a Developer ID identity + a `notarytool submit` + `stapler` step — no rework.
- **Sidecar production resolution (D6).** `pick_asr` / `pick_llm` gain a third tier: **env var (dev/CI) → sibling-of-engine-binary (production, via `std::env::current_exe()`) → Rust fallback** (whisper-rs / Ollama). No env var in production.
- **Models: download-on-first-run + mirror (#8).** Keep the first-run download; point `model.rs` at a **GitHub Release asset** on the panops repo (free, stable, project-controlled). Upstream HF/GitHub stays a documented fallback.
- **North-star #6 amended** (see below).

## Scope

### In
- **Engine sidecar self-resolution:** extend `asr_resolver::pick_asr` + `llm_resolver::pick_llm` with the `current_exe()`-sibling tier (reusing `sidecar_binary.rs`'s canonicalize + exec-bit validation). Unit-tested.
- **Model mirror (#8):** `crates/panops-portable/src/model.rs` — Whisper (and diar/VAD) URLs point at the GitHub Release mirror; document the upstream fallback. Hashes unchanged (sha256 still verifies).
- **`scripts/package.sh`:** build engine (`cargo build --release`) + app + both sidecars (`swift build -c release`) → assemble `Panops.app/Contents/{MacOS,Resources}` → write `Info.plist` (bundle id `dev.panops.Panops`, version, `NSMicrophoneUsageDescription`) + `Panops.entitlements` (mic `com.apple.security.device.audio-input`) → `codesign --options runtime -s -` (ad-hoc, hardened runtime) the sidecars/engine then the outer `.app` → produce `Panops-<version>.tar.gz` + its sha256. Idempotent, no secrets.
- **Homebrew tap:** a `homebrew-panops` cask (`cask "panops"`) pointing at the GitHub Release tarball + sha256, with `caveats` for the one-time quarantine removal, installing to `/Applications`.
- **CI:** run `package.sh` (build + assemble + ad-hoc sign; **no** notarize) on `push: main` so bundle/signing breakage is caught.
- **Tests:** unit test for the `current_exe`-sibling resolution tier (both resolvers); a smoke test that an assembled bundle's engine resolves the bundled sidecars (not the fallback).

### Out (file as debt if surfaced)
- **Paid Developer-ID notarization channel** (`notarytool` + `stapler` + a notarized-`.app`/official-cask) — lands when a paid account exists.
- **Auto-update** (Sparkle or similar).
- **#160** (CI build/test of the sidecars) — tracked separately.
- **Screen-recording capture entitlement/UX** — real ScreenCaptureKit capture is a future slice; screen recording is a runtime TCC grant (no paid entitlement), wired when capture lands.
- **Universal binary / Intel** — Apple Silicon only for v0.1 unless trivial.

## Architecture

```
Panops.app/Contents/
  MacOS/Panops               ← SwiftUI app (swift build -c release)
  Resources/panops-engine    ← Rust engine (cargo build --release)
  Resources/panops-asr-mac   ← WhisperKit sidecar (swift build -c release)
  Resources/panops-llm-mac   ← FoundationModels sidecar (swift build -c release)
  Info.plist, Panops.entitlements

Launch + resolution:
  Panops (app) --Bundle.main/Contents/Resources/panops-engine--> spawns engine   [already implemented]
  engine --current_exe() dir → panops-asr-mac / panops-llm-mac--> spawns sidecars [NEW: D6 tier]
    resolution order per resolver: $PANOPS_*_SIDECAR_BIN (dev/CI)
                                 → <dir of current_exe>/panops-*-mac (production)
                                 → Rust fallback (whisper-rs / GenaiLlm-Ollama)
  engine --first run--> downloads models from GitHub-Release mirror (#8), sha256-verified
```

## Data flow (first launch on a clean Mac)
1. `brew install --cask vfmatzkin/panops/panops` → `Panops.app` in `/Applications` (quarantined).
2. User clears quarantine once (per `caveats`) and launches.
3. App spawns the bundled engine (`Bundle.main`); engine self-locates the bundled sidecars (`current_exe` sibling).
4. First note/transcript triggers a model download from the mirror (sha256-verified, cached in Application Support); subsequent runs reuse it.
5. FoundationModels LLM + WhisperKit ASR run on-device via the bundled sidecars.

## North-star amendment (criterion #6)
Current: *"Build + sign + notarize the `.app`; runs on a clean Mac with no dev tools."*
Amended for v0.1: *"Ad-hoc-signed `.app`, **brew-installable** (`brew install --cask vfmatzkin/panops/panops`), runs on a clean Mac that has Homebrew; a Developer-ID-**notarized** `.app` is a later distribution channel once a paid account exists."*
Rationale: zero feature difference (same capture/ASR/LLM/notes); only distribution trust differs. Maintainer to ratify in `north-star.md` when the slice merges.

## Three-tier boundaries

### ✅ Always do
- `cargo fmt --all && cargo build --workspace --locked && cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings`; `swift build -c release` for app + both sidecars.
- Keep `panops-core` platform-free; resolver changes live in `panops-engine` (+ `#[cfg(target_os="macos")]` where needed).
- Open issues for every deferred item; commit per plan task; verify pushed == local.
- Sign with hardened runtime + entitlements (notarization-ready) even for the ad-hoc build.

### ⚠️ Ask first
- Changing the bundle id, the entitlements set, or adding any entitlement beyond mic.
- Re-hosting model weights anywhere other than the panops GitHub Releases (licensing).
- Adding a new runtime dependency to `panops-core` / `panops-portable`.
- Removing the dev/CI env-var resolution path (keep it for tests).

### 🚫 Never do
- Telemetry / network calls beyond the model download (which hits only the documented mirror + sha256-verifies).
- A user-facing env var for production config (the sidecar env gates stay dev/CI-only).
- Bundle a paid-account-gated entitlement (iCloud, Push, App Groups, Network Extension) — none are needed.
- Open or merge the PR autonomously; commit the maintainer's signing secrets anywhere (the ad-hoc build needs none).

## Acceptance criteria
1. `scripts/package.sh` produces a launchable `Panops.app` whose engine resolves the **bundled** sidecars (not the Rust fallback) with no env vars set.
2. Installed via the tap cask on a clean Mac (with Homebrew), after the documented one-time quarantine removal the app launches, downloads models from the mirror on first run, and generates notes on-device via the FoundationModels sidecar.
3. `cargo test --workspace` green incl. the new `current_exe`-sibling resolution test; CI's `package.sh` build+assemble+ad-hoc-sign step passes.
4. No telemetry; no new user env vars; model download hits only the mirror and sha256-verifies.

## Risks
- **Ad-hoc TCC reset per version** — the mic/screen grant re-prompts on each upgrade (accepted by the maintainer; the paid stable identity later fixes it).
- **`current_exe()` + symlinks** — Homebrew/`/Applications` may symlink; resolver must `canonicalize` before locating siblings (the existing `sidecar_binary.rs` already canonicalizes).
- **GitHub Release asset size** — 547 MB Whisper is within the 2 GB asset limit; document the upload step in `package.sh`.
- **Clean-Mac acceptance** needs a VM / fresh user; manual gate.

## Open questions (deferred)
- Tap repo layout: standalone `homebrew-panops` repo vs a `Casks/` dir — settle in writing-plans (default: standalone tap repo).
- Whether to ship `ggml-base-q5` (57 MB, faster install) as the default vs `large-v3-turbo` (547 MB, best quality) — tie to the calibration slice (#14).
