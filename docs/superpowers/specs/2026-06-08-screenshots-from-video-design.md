# Screenshots from video — single screenshot source via frame extraction — design

**Date:** 2026-06-08
**Status:** Drafted autonomously for maintainer review (maintainer approved doing this slice "after" the editing work, high-level). **Spec + the foundation stage build first; the engine-rewire stage waits for maintainer review of this spec.**
**Advances:** consolidation the maintainer asked for — "we have both video recording and screenshots; maybe screenshots should come from the video, not two systems."

## Goal

Today there are **two independent screen-capture systems**: a live `Screenshotter` (samples frames during recording with Vision FeaturePrint change-detection → screenshot JPEGs) and ScreenCaptureKit video recording (→ `recording.mov`). When both run, the screen is sampled twice. Consolidate to **one source of truth**: when video is recorded, derive the screenshot anchors by extracting + change-detecting frames from `recording.mov`. Keep the live `Screenshotter` only as a **fallback when video is off** (audio-only meetings still get anchors).

Benefits: one capture path; screenshots are exactly consistent with what was recorded; fits the deferred-processing model (extract during processing, then the video can be deleted). Trade-off (accepted): screenshots inherit the video's resolution.

## Approach

Reuse the existing change-detection; only the frame **source** changes (live SC frames → decoded `.mov` frames).

1. **Factor out the detector.** Extract the Vision FeaturePrint cosine-distance change-detection out of the live `Screenshotter` (`apps/panops-capture-mac/Sources/PanopsCaptureMac/Screenshotter.swift`) into a reusable unit both the live path and the extract path call. Keep its threshold/interval semantics identical.
2. **`--extract-screenshots` sidecar command.** Add a one-shot mode to `panops-capture-mac`: `--extract-screenshots <mov> <out_dir> --interval-ms N --threshold T`. It decodes frames at the interval (AVAssetImageGenerator / AVFoundation), runs the shared detector, writes changed frames as JPEGs (same naming + output shape as the live `Screenshotter`), and prints the screenshot metadata (timestamp_ms + path per kept frame) as JSON.
3. **Engine wires the video path.** When a recording **with video** finishes, the engine invokes `--extract-screenshots` on the `recording.mov` (as part of post-recording processing / before notes generation) to produce the screenshot set, instead of relying on live screenshots. When video is **off**, the engine keeps the live `Screenshotter` path unchanged (fallback).
4. **Don't double-sample.** During a video recording, the live `Screenshotter` is **not** run (one system when video is on).

## Where extraction runs

For this slice: at post-recording processing (engine side, after `recording.stop`, before/with notes generation), gated on "video was recorded." The full deferred-processing model (record now, process later, delete video) is a **separate slice** — this slice just makes screenshots come from the video when both exist, so it composes cleanly with deferred processing later.

## Build order (staged PRs)

- **Stage A — foundation (safe, isolated; build now):** factor the change-detector into a reusable unit (with unit tests on the detector) + add the `--extract-screenshots` sidecar command + a test that runs it on a fixture `.mov` and asserts it writes the expected changed frames. **No engine/pipeline change yet** — purely additive (a new command + a refactor). Verifiable in isolation.
- **Stage B — engine rewire (hold for maintainer review of this spec):** wire post-recording processing to call `--extract-screenshots` for video recordings; skip the live `Screenshotter` when video is on; keep it as the no-video fallback. This changes runtime behavior, so it lands after the maintainer reviews the spec.

## Three-tier boundaries
- ✅ Always: `cargo fmt`/`clippy` + `swift build`/`swift test`; reuse the existing detector (identical threshold/interval); unit-test the detector + the extract command on a fixture `.mov`; open issues for deferred items.
- ⚠️ Ask-first (Stage B): changing when/where screenshots are produced in the pipeline; removing the live `Screenshotter` from the video path; any change to screenshot output naming/format consumed by notes.
- 🚫 Never: SaaS-isms; telemetry; dropping screenshot anchors for audio-only meetings (keep the live fallback); deleting the user's `recording.mov` (that's the deferred-processing slice's concern, opt-in); user-config env vars.

## Verification
- Swift: `swift build` + `swift test` for the detector unit + the `--extract-screenshots` command on a fixture `.mov` (use/extend `tests/fixtures/`).
- Stage B (later): an engine test that a video recording yields screenshots from the `.mov`, and an audio-only recording still yields live screenshots.
- Manual smoke (signed bundle, after Stage B): record with video → notes show screenshots that match the recording.

## Out of scope (deferred → debt)
- The full deferred-processing model (record now / process later / delete video) — separate slice.
- Deleting `recording.mov` after extraction (opt-in, deferred slice).
- Cursor/click highlights, presenter overlay.
- Re-extracting at a different interval/threshold after the fact.
