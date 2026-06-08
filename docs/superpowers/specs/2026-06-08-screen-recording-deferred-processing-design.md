# Slice: Screen recording + deferred processing — design

**Date:** 2026-06-08
**Status:** Approved (maintainer approved the design in brainstorm; authorized autonomous build).
**Advances:** Anchor B (live capture) — makes `recording.start` actually capture, and decouples capture from compute.

## Goal

Let the user record a meeting (full screen or a window) as a kept video, and choose whether to run the AI pipeline now or defer it — so capture works even when compute is unavailable (Apple Intelligence off, Ollama down, low battery). Large video is deletable afterward to reclaim space, without touching the notes.

## Why (north-star alignment)

- Serves v0.1 criteria **#1/#2** (capture → audio + screenshots → transcript) and the maintainer's real workflow (his pre-panops flow was OBS-record → transcribe → notes).
- **Local-first, no compute required to capture.** The capture sidecar needs no LLM/ASR; only `notes.generate` does, and it stays on-demand.
- **No SaaS-isms, zero telemetry, MIT.** Everything on-device. The video is a local file.
- **Multilingual preserved** (language flows through `MeetingConfig` as today).

## Two orthogonal user choices (New Recording sheet)

1. **What to capture** — audio + screenshots always (they drive notes); **video** on/off; target = **full display** or **a window**.
2. **When to process** — **Record + notes** (run the AI pipeline after stop; degrades to deferred if compute is unavailable) or **Record only** (capture, skip AI; the meeting waits as "Needs notes" until processed later).

"No compute now? → Record only (+ video)." "Normal meeting → Record + notes."

## Architecture

**Augment the existing capture stream.** `panops-capture-mac` already runs one `SCStream` whose screen frames feed the `Screenshotter` and whose audio feeds the WAV writers. Add an `AVAssetWriter` fed by the *same* stream → `recording.mov`. One stream, one set of permissions; video is purely additive. The `SCContentFilter` selects a full display or a specific window — and because it's the same stream, the screenshots that anchor notes match what's recorded.

Processing stays decoupled: `notes.generate` (ASR → diarization → notes) is the only compute step and remains an on-demand job. "Record + notes" simply means the engine auto-kicks `notes.generate` after `recording.stop`; "Record only" doesn't.

The notes are built from the clean system+mic audio + screenshots, **not** the video — so deleting the video never affects the transcript or notes.

## Wire contract (source of truth for the parallel tracks)

All snake_case on the wire (per repo convention). Swift mirrors with explicit `CodingKeys`.

- **`ipc.recording.start`** — `RecordingStartParams` gains:
  - `record_video: bool` (default `false`) — write `recording.mov` for this session.
  - `capture_target: CaptureTarget` (default `{ "kind": "display" }`) — `{ "kind": "display" }` or `{ "kind": "window", "window_id": <u32> }`.
  - `auto_generate_notes: bool` (default `false`) — engine auto-runs `notes.generate` after `recording.stop` succeeds.
- **`ipc.capture.windows`** → `{ "windows": [ { "window_id": <u32>, "app_name": String, "title": String } ] }` — shareable on-screen windows for the picker (excludes panops's own windows; desktop-only, on-screen).
- **`ipc.meeting.deleteVideo`** — params `{ "meeting_id": String }` → `{ "deleted": bool, "freed_bytes": u64 }`. Deletes `recording.mov` from the meeting dir; idempotent (no file → `deleted:false, freed_bytes:0`). Does **not** touch transcript/notes/audio/screenshots.
- **Video artifact convention:** `~/Library/Application Support/panops/meetings/<uuid>/recording.mov`. The app discovers it on disk (like `notes.json`/`transcript.json`) — no path field on the wire. Size via filesystem stat.

## Components

**`apps/panops-capture-mac` (Swift sidecar):**
- New video output: an `AVAssetWriter` (`.mov`, H.264 or HEVC) with a video input fed from the `SCStream` `.screen` sample buffers and an audio input from the system/mic samples. Started only when `record_video` is set; finalized on stop. Reuses the running stream (no second `SCStream`).
- `SCContentFilter` from `capture_target`: full `SCDisplay`, or a specific `SCWindow` by id.
- A `capture.windows`-style enumeration via `SCShareableContent` (for the engine method).
- Robustness: if the video writer fails, log + continue audio/screenshots (video is best-effort; never abort the capture).

**`crates/panops-engine` + `crates/panops-protocol` (Rust):**
- Protocol: the three wire additions above (params/results), with domain↔wire conversions; conformance updated. Domain error types stay non-`Serialize`.
- Engine: pass `record_video` + `capture_target` to the sidecar; implement `capture.windows` (delegates to the capture port) and `meeting.deleteVideo` (filesystem delete under the meeting dir, path-validated). On `recording.stop`, if `auto_generate_notes`, enqueue `notes.generate` (reusing the existing job path + events) only if a provider is resolvable; otherwise leave the meeting as needs-notes and log.

**`apps/Panops` (Swift app):**
- New Recording sheet: **Record video** toggle; **target** picker (Full display / a window via `capture.windows`); **processing mode** (Record + notes / Record only) → maps to `auto_generate_notes`.
- Meeting detail: when `recording.mov` exists, show a Recording row with its size + **Play** (open in default player) / **Reveal in Finder** / **Delete video to reclaim space** (calls `meeting.deleteVideo`, confirms, refreshes size). Deferred meetings keep the existing "Needs notes" + Generate action.
- Decode the new params; mirror `CaptureTarget`/window list with explicit `CodingKeys`.

## Storage

`recording.mov` lives in the meeting dir alongside `audio*.wav`, screenshots, `transcript.json`, `notes.{md,json}`. Deleting it is a pure file removal; everything else is independent. No DB schema change required (discovered on disk); the registry is untouched.

## Build order / PR split

- **PR 1 — walking skeleton (record-only, full display, deletable).** `record_video` flag end-to-end: sidecar `AVAssetWriter` (display only) + engine pass-through + `meeting.deleteVideo` + app (Record-video toggle, detail Play/Reveal/Delete). No window targeting, no auto-process (always record-only this PR). Delivers the core value: record a video, watch it, delete it.
- **PR 2 — processing mode + window targeting.** `auto_generate_notes` (auto-run notes after stop, degrade gracefully) + `capture_target` window support + `capture.windows` + the sheet's target picker + processing-mode control.

Each PR = its own branch/PR; maintainer merges. Within PR 1, the three surfaces (Rust / sidecar / app) are isolated directories sharing only the pinned wire contract above, so they're built in parallel and integrated.

## Three-tier boundaries

- ✅ **Always do** — run `cargo fmt`/`clippy` + `swift build`/`swift test` (CLT framework paths) before pushing; commit per logical unit; open issues for deferred items; verify socket-binding tests locally (codex's sandbox can't); keep the augment invariant (notes built from audio+screenshots, never the video).
- ⚠️ **Ask first** — any DB schema/registry migration; changing the recording lifecycle (`recording.start/stop`) contract beyond the additions above; bundling/shipping decisions (packaging is a separate slice); adding a real "screenshots-off" path (that's #213's domain).
- 🚫 **Never do** — a second `SCStream` for video (reuse the one); make notes depend on the video; introduce telemetry/phone-home; add user-config env vars (sidecar bins stay dev/CI escape hatches); auto-merge a PR; weaken the local-first default (cloud LLM stays explicit + surfaced).

## Verification

- **Rust:** `cargo build/test/clippy --workspace --locked` (socket tests verified in a normal shell by the primary session, not codex's sandbox).
- **Sidecar + app:** `swift build` + `swift test` with the CLT framework rpaths (`-Xswiftc -F .../CommandLineTools/.../Frameworks`, etc.).
- **Integration (primary session):** build the engine + run the app in dev with `PANOPS_CAPTURE_SIDECAR_BIN` set to the built sidecar + screen/mic permissions granted; record → confirm `recording.mov` plays + screenshots/audio still captured; delete → confirm space freed + notes/transcript intact. (Full on-device shipping needs the packaging slice; not in scope here.)

## Out of scope (deferred → debt)

- Packaging / code-signing / screen+mic permission entitlements (slice 16; prerequisite to *ship* live capture, not to build it).
- Notarization (north-star: later channel).
- Screenshots-off path (#213).
- Importing externally-recorded video → notes (complement to "Open audio file…"; future).
- Live transcript during recording (north-star: not required for v0.1).
