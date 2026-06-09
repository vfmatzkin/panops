# Slice: Capture confidence — picker, preview, region/resolution, live monitoring — design

**Date:** 2026-06-08
**Status:** Approved for build (maintainer brainstormed + approved 2026-06-08).
**Advances:** Anchor B (live capture). Makes capture *trustworthy* — you can see what will be captured before you start, and that audio+video are actually flowing while you record.

## Goal

Today recording is a black box: you hit Record and hope. Make capture predictable and verifiable end to end. Concretely, kill four failure modes the maintainer named:

1. **Recorded nothing** — ran a whole meeting, got an empty/silent file.
2. **Captured the wrong thing** — wrong window/screen, or grabbed the full screen incl. private content.
3. **Wrong quality/size** — too low to read, or a huge file with unwanted screen area.
4. **Can't tell what's happening** — no feedback during capture; pure black box.

Single-user, on-device, no telemetry. (Frames previewed locally never leave the machine.)

## Product model

A **capture selection** describes exactly what gets recorded. The user builds it in the New Recording setup with a live preview, and it is honored verbatim by the recording pipeline.

- **Source** — picked via Apple's system `SCContentSharingPicker`: a **Display**, a **Window**, or an **App**.
- **Region** — an optional sub-rectangle of a display, set by **dragging a crop box on the live preview** (maps to `SCStreamConfiguration.sourceRect`). Window/App capture is already cropped to that surface; region applies to display capture.
- **Resolution** — output dimensions from a preset dropdown: **Native / 1080p / 720p / 480p**, showing the resulting WxH (maps to `SCStreamConfiguration.width`/`height` with `scalesToFit`).
- **Audio sources** — System+Mic / System / Mic (already exists).
- **Screenshots on/off** (already exists).

## Architecture (decision: "App previews, sidecar records")

The app renders the smooth preview from its **own** `SCStream`; the capture **sidecar** records the **identical** selection to file and reports live health. Two streams of the same source, kept in sync by a serialized selection contract. Rationale vs. alternatives (sidecar-streams-preview; app-owns-all-capture) is in the brainstorm; this keeps the engine/sidecar ownership from slice 11 intact and makes smooth preview + a native picker nearly free.

```
┌─ Panops.app ──────────────────────────────┐         ┌─ panops-engine ─┐   ┌─ panops-capture-mac ─┐
│ SCContentSharingPicker → CaptureSelection │  IPC    │ recording.start │   │ rebuild SCContentFilter│
│ in-app SCStream → AVSampleBufferDisplay-  │ ──────► │  {selection}    │──►│ + SCStreamConfiguration│
│   Layer  (smooth preview + drag-crop)     │         │                 │   │ → record .mov + wavs   │
│ meters + health readout  ◄────────────────┼─events──┤ capture.levels  │◄──┤ RMS per buffer (~15Hz) │
│                                           │         │ recording.health│◄──┤ bytes/frames (~1–2Hz)  │
└───────────────────────────────────────────┘         └─────────────────┘   └───────────────────────┘
```

**The selection contract.** `SCContentFilter` can't cross processes, so the app extracts a serializable `CaptureSelection` from the picker result and the sidecar rebuilds an equivalent filter via `SCShareableContent` lookup. Supported selections are constrained to the serializable cases below; anything the picker returns outside them is rejected with a clear message rather than silently recording the wrong target.

## Data contract — `CaptureSelection`

Extends the capture port (`crates/panops-core/src/capture.rs`). Today `CaptureTarget` is `Full | Window{window_id}`; widen to:

```
CaptureSelection {
  target: Target,                 // see below
  width:  Option<u32>,            // output px; None = native
  height: Option<u32>,            // (width,height) both None or both Some
  audio_sources: AudioSources,    // existing
  capture_screenshots: bool,      // existing
}
Target =
  | Display { display_id: u32 }
  | Window  { window_id: u32 }
  | App     { bundle_id: String }
  | Region  { display_id: u32, x: u32, y: u32, w: u32, h: u32 }   // sourceRect on a display
```

`Full`/`Window{id}` map forward (Full → primary `Display`). Wire type mirrors the domain in `panops-protocol` with From conversions both ways + round-trip tests. Domain error types stay non-`Serialize` (project rule).

## IPC (`panops-protocol` + engine handlers)

- `recording.start` params gain the full `CaptureSelection` (replacing the bare `capture_target`; keep a back-compat default so an absent selection = full primary display + native + system+mic, matching today).
- **Events** over the existing event channel (same shape as `recording.progress`):
  - `capture.levels { system_db: Option<f32>, mic_db: Option<f32> }` — ~15 Hz, for meters.
  - `recording.health { bytes_written: u64, video_frames: u64, elapsed_ms: u64 }` — ~1–2 Hz, the "it's actually being written" proof.
- `capture.windows` (existing engine-side enumeration) becomes vestigial for picking (the app uses `SCContentSharingPicker`); leave it in place, mark deprecated in a comment. Do not remove this slice.

## App UI (`apps/Panops`)

**New Recording setup (extend `NewRecordingSheet.swift` + new views):**
- "Choose what to capture" → opens `SCContentSharingPicker`; result drives the preview.
- **Preview pane** — `NSViewRepresentable` over a view whose `AVSampleBufferDisplayLayer` is fed by an in-app `SCStream` (`stream(_:didOutputSampleBuffer:)`). Smooth, hardware-accelerated.
- **Drag-crop overlay** on the preview → `Region` rect; live-updates the preview framing.
- **Resolution dropdown** (Native/1080p/720p/480p) showing resulting WxH.
- Existing controls (title, language, audio source, screenshots) stay.

**Recording screen (promote `RecordBar.swift` → a real recording view):**
- Keeps the **same preview** live during recording (the app's preview `SCStream` keeps running).
- **Mic + System meters** (pre-flight: from the preview stream's audio tap; during recording: from `capture.levels`, i.e. the real pipeline).
- **Health line**: "● recording — MM:SS · 1.2 GB · 4021 frames" from `recording.health`.
- Prominent **Stop**; trust strip ("Local · Private").

## Sidecar (`apps/panops-capture-mac`)

- `Recorder` honors `sourceRect` (region) + `width`/`height` (resolution) on its `SCStreamConfiguration`, and rebuilds the `SCContentFilter` for Display/Window/App/Region from the serialized selection via `SCShareableContent`.
- In the existing `SCStreamOutput` audio path, compute per-buffer RMS → dBFS and emit `capture.levels`. In the video/writer path, emit `recording.health` (bytes on disk + frame count) on a timer.
- Pure pieces unit-tested off-device: `CaptureSelection → SCStreamConfiguration`/filter-params mapping; RMS→dBFS; the resolution-preset → (w,h) math.

## Permissions

The app now runs its own preview `SCStream`, so the **app process** needs Screen Recording (today only the sidecar does). Same signed bundle; it prompts once on first preview. The setup sheet must handle "not yet granted" with a clear CTA + a retry after the grant (no silent blank pane).

## Build order (one combined slice → one PR, fleet-built, dual-model-reviewed)

Within the single slice, land in this internal order so each step is independently testable:
1. **Data contract** — `CaptureSelection` domain + wire + conversions + round-trip tests (Rust). No behavior change yet.
2. **Sidecar honors selection** — region/resolution + filter rebuild + pure-mapping unit tests (Swift).
3. **Sidecar emits events** — `capture.levels` + `recording.health` + fake/conformance (Rust+Swift).
4. **App pre-flight** — picker + preview pane + drag-crop + resolution, wired to `recording.start`.
5. **App live** — preview during recording + meters + health line.

## Three-tier boundaries

- ✅ **Always:** `cargo fmt`/`clippy` + `swift build`/`swift test` (CLT framework paths) before pushing; commit per build-order step; unit-test every pure mapping; open issues for any deferred item; keep `recording.start` back-compatible (absent selection = today's behavior).
- ⚠️ **Ask-first:** removing/renaming `capture.windows` or the existing `capture_target` field (back-compat shim instead); changing the on-disk recording format/codec; adding a *second* concurrent audio capture during recording if CPU proves a problem (vs. switching meter source).
- 🚫 **Never:** SaaS-isms; telemetry or sending preview frames anywhere off-device; a new port/trait without a real impl + fake; opening/merging a PR autonomously; user-config env vars.

## Verification

- Rust: `cargo build/test/clippy --workspace --locked` incl. socket tests; `CaptureSelection` wire round-trips; events delivered through the fake in the capture conformance suite.
- Swift: `swift build` + `swift test` (CLT paths); pure mapping/RMS/preset tests.
- Manual smoke (signed bundle, `scripts/package.sh` → `open dist/Panops.app`): pick a window → preview matches → drag-crop → set 720p → Start → preview stays live, meters move, health bytes climb → Stop → file contains exactly the cropped/scaled region with audio. Also: pick a display, full; pick an app.

## Out of scope (deferred → file as debt)

- Multi-source / scene composition (OBS-style layering). One source per recording.
- Per-window audio isolation beyond the existing system/mic split.
- Saved capture presets / remembering last selection across launches.
- Pause/resume mid-recording.
- Picker selections that aren't serializable to a single Display/Window/App/Region (e.g. multi-window exclusion sets) — rejected with a message, not supported.
- Cursor/click highlight, presenter overlay.
