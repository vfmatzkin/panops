---
project: panops
date: 2026-04-30
status: design (pre-implementation)
authors: Fran Matzkin
type: design
---

# panops — design

## Summary

panops is an open, local-first macOS recorder that captures audio (mic + system + per-app), screen, and time-anchored screenshots, transcribes live, refines with a higher-quality post-pass on the saved media, and emits markdown meeting notes with embedded screenshots via a BYO local-or-cloud LLM. The wedge — verified unclaimed in the OSS landscape as of 2026-04-30 — is **screenshot-anchored notes** built on a portable engine, with a thin native macOS shell. No accounts, no cloud-required path, no per-meeting cap.

## Goals

- Replace a manual OBS → batch-transcript → manual-notes chain with a single local app.
- Live transcription during the meeting, plus a higher-quality second pass on the saved file, plus diarization on the post-pass.
- Multilingual day 1 (EN / ES / IT first; Whisper covers 99). One-tap language toggle for bilingual meetings, including mid-meeting switching.
- Native macOS performance via WhisperKit + FluidAudio + ScreenCaptureKit.
- Portable engine: same Rust binary runs headless on Linux/Windows once non-Mac capture and ASR adapters are added; Mac-only code is one feature flag away.
- Hexagonal architecture; every platform-specific concern is a port with at least two adapter implementations (or one + a clear extension point) so the boundary is real, not aspirational.
- BYO-everything: local Whisper / Parakeet / Apple FoundationModels / Ollama / llama.cpp; cloud OpenAI / Anthropic / Groq / OpenRouter behind one trait.
- Output is plain markdown + embedded screenshot files. Drop-in for Obsidian-style folders. SRT / VTT / TXT / JSON also exported.

## Non-goals

- Always-on screen recall (Screenpipe owns that).
- Cloud-hosted team workspace (Granola, Otter own that).
- System-wide dictation / push-to-talk (Superwhisper, MacWhisper own that).
- Joining meetings as a bot (BB Recorder / Granola own that; we record the local audio output instead).
- Real-time translation (post-pass only, if at all).
- Mobile clients.

## Architecture

Hexagonal / Ports & Adapters in Rust. The core domain has zero platform code. Every OS- or vendor-specific concern is a trait. Each trait has a `mac-native` adapter and a `portable` adapter. Cargo features select which compiles. Compile-time selection is the primary mechanism; an optional `Fallback<T>` decorator (off by default, enabled in shipping config for the ASR port only) retries the secondary at runtime if the primary errors.

```
                    ┌────────────────────────────┐
                    │     Panops.app (SwiftUI)   │  ← Mac shell, thin client
                    │  capture · UI · screenshots│
                    └──────────────┬─────────────┘
                                   │ JSON-RPC (control) + WebSocket (events)
                                   │ over Unix domain socket
                    ┌──────────────▼─────────────┐
                    │     panops-engine (Rust)   │  ← portable, headless-capable
                    │  use cases · orchestration │
                    └──────────────┬─────────────┘
                                   │ traits (ports)
        ┌──────────────────────────┼──────────────────────────┐
        ▼                          ▼                          ▼
   AsrProvider              AudioCapture               LlmProvider
   ┌──────────┐             ┌────────────┐            ┌────────────┐
   │ mac:     │             │ mac:       │            │ mac:       │
   │ panops-  │             │ ScreenCap- │            │ Foundation │
   │ asr-mac  │             │ tureKit    │            │ Models     │
   │ (Swift   │             │ (objc2)    │            │ (Swift)    │
   │ sidecar) │             ├────────────┤            ├────────────┤
   ├──────────┤             │ portable:  │            │ portable:  │
   │ portable:│             │ cpal       │            │ rust-genai │
   │ whisper- │             │            │            │ (Ollama,   │
   │ rs       │             │            │            │ OpenAI,    │
   │          │             │            │            │ Anthropic, │
   │          │             │            │            │ Groq, …)   │
   └──────────┘             └────────────┘            └────────────┘
```

The Swift sidecar `panops-asr-mac` is a tiny binary that wraps WhisperKit (Argmax Pro SDK 2) and FluidAudio (CoreML/ANE Parakeet TDT v3). It exposes one stdin/stdout protocol: receive PCM frames, emit JSON segments. The Rust engine treats it like any other `AsrProvider` adapter. On Linux/Windows the sidecar isn't built; the engine uses `whisper-rs` with whatever backend (CUDA, Vulkan, CPU).

## Repo layout

```
panops/
├── Cargo.toml                      (workspace, resolver = "2")
├── README.md
├── LICENSE                          (MIT)
├── crates/
│   ├── panops-core/                 domain types, ports (traits), use cases. zero platform code.
│   ├── panops-portable/             impls of every port that work everywhere
│   ├── panops-mac/                  #[cfg(target_os="macos")] impls (objc2 + sidecar runners)
│   ├── panops-engine/               binary: wires ports → adapters, IPC server, CLI mode
│   └── panops-protocol/             IPC schema (serde types, JSON-RPC method definitions)
├── apps/
│   ├── Panops/                      SwiftUI macOS app (Xcode project)
│   └── panops-asr-mac/              Swift Package binary: WhisperKit + FluidAudio sidecar
├── proto/
│   └── ipc.md                       human-readable protocol doc (mirrors panops-protocol)
├── docs/
│   ├── superpowers/specs/           design docs (this file)
│   └── research/                    market scan, verification reports
├── tests/
│   └── adapter-conformance/         every adapter passes the same trait test suite
└── scripts/
    └── package.sh                   builds .app, signs, notarizes
```

## Adapter table

Verified 2026-04-30. URLs cited inline only on swaps; full citation list in `docs/research/2026-04-30-market-scan.md`.

| Port | mac-native adapter | portable adapter | Notes |
|---|---|---|---|
| `AsrProvider` (live) | `panops-asr-mac` Swift sidecar — **Parakeet TDT v3 via FluidAudio (CoreML/ANE)**, ~110× RTF on M4 Pro, ~2.5× faster than Parakeet-MLX (FluidAudio v0.14.3, Apr 2026). | `whisper-rs` 0.16.0 wrapping whisper.cpp 1.8.4 (Metal on Mac, Vulkan/CUDA/CPU elsewhere). | Default ASR is the live path on incoming PCM frames. Language hint per chunk. |
| `AsrProvider` (post-pass) | Same Swift sidecar, **whisper-large-v3-turbo via WhisperKit (Argmax Pro SDK 2)** with `beam_size=5`, VAD-gated. | `whisper-rs` with whisper-large-v3 (full, not turbo), beam=5. | Optional Mistral **Voxtral-Mini-3B** via `voxtral.c` MPS as a quality alternative on long meetings. |
| `Diarizer` | FluidAudio diarizer (CoreML, pyannote-derived weights). | `pyannote.audio Community-1` via `ort` 2.x with CoreML EP on Mac, CPU elsewhere. | Runs on post-pass only. Heavy; downloaded on first use, not bundled. |
| `AudioCapture` | ScreenCaptureKit (mic + system audio + per-app, macOS 14+) via `objc2` + `screencapturekit-rs`. | `cpal` (mic only — Linux/Win system-audio capture is OS-specific, deferred). | One unified API on Mac avoids BlackHole / Loopback. |
| `VideoCapture` | ScreenCaptureKit screen / window / display. | `xcap` (Linux X11 + Wayland, Windows DXGI, video record WIP). | Replaces unmaintained `scrap`. |
| `Encoder` | AVFoundation hw H.264/HEVC via Swift sidecar or objc2. | `ffmpeg` subprocess (simplest), `gstreamer-rs` if a pipeline DSL is needed later. | |
| `ScreenshotSampler` | `VNGenerateImageFeaturePrintRequest` (Apple Vision feature-print embedding) via objc2. | `image_hasher` 3.x (maintained fork; original `img_hash` is dead). | Visual-change detection only; not a recall index. |
| `LlmProvider` | `AppleFoundationModelsAdapter` (macOS 26 Tahoe, ~3B on-device) called from a small **separate** Swift sidecar (`panops-llm-mac`, distinct from the ASR sidecar to keep concerns separated and avoid loading WhisperKit when only the LLM runs). | `rust-genai` (Ollama / OpenAI / Anthropic / Gemini / Groq / DeepSeek / xAI / Cohere behind one trait). | Provider chosen per template; user picks default. |
| `Storage` | — *(none planned; SQLite is portable already)* | `rusqlite` (single-user local-first; sqlx async overhead unneeded). | One SQLite per meeting + media files in `~/Library/Application Support/panops/meetings/<id>/`. The trait still exists so a future `PostgresStorage` or cloud-sync adapter can drop in. |
| `Vad` | Silero VAD via Apple's Core ML runtime through the Swift sidecar. | `Silero` ONNX via `ort` 2.x. | Internal to the live pipeline; gates ASR windows so we don't burn cycles on silence. |

## Live + post-pass pipeline

A single `AsrProvider` trait exposes both methods — `transcribe_window` for live and `transcribe_full` for post-pass. Each adapter decides whether the two methods share a model (e.g. portable adapter uses `whisper-large-v3-turbo` for both) or use different models per method (e.g. mac-native uses Parakeet for live, whisper-large-v3-turbo for post-pass).

**Live pass** (during recording):
1. Mac shell captures audio with ScreenCaptureKit at native sample rate, downsamples to 16 kHz mono for ASR, streams 20 ms PCM frames over Unix socket to engine.
2. Engine runs `Vad::should_emit(window)` — silent windows are skipped before ASR.
3. Engine batches frames into a ring buffer (configurable, default 5 s window with 1.5 s overlap).
4. On each non-silent window, engine calls `AsrProvider::transcribe_window(pcm, language_hint)` which forwards to the Swift sidecar; sidecar runs Parakeet TDT v3 on ANE.
5. Sidecar emits `Segment { start, end, text, language_detected, confidence, is_partial }`.
6. Engine deduplicates overlap, merges into the current `Meeting.live_transcript`, emits to UI via WebSocket.

**Post-pass** (after recording stops):
1. Engine reads the saved audio file (already on disk, never crossed IPC during the meeting).
2. Calls `AsrProvider::transcribe_full(file_path, language_hint, mode=highest_quality)`.
3. Sidecar runs whisper-large-v3-turbo via WhisperKit with `beam_size=5`, VAD filter, `condition_on_previous_text=true`.
4. Engine reconciles: post-pass transcript replaces live transcript; live segment IDs are preserved where text matches (so live edits don't get clobbered).
5. `Diarizer::label_speakers(audio_path, segments)` runs FluidAudio diarizer; assigns `speaker_id` per segment.
6. User reviews segments + speaker labels in UI, can rename speakers (`Speaker 1` → `Fran`), edit text.
7. `LlmProvider::summarize(transcript, screenshots, template)` produces markdown notes with screenshot embeds.

## Multilingual & language toggle

State machine:

```
Language ::= Auto | En | Es | It | <iso639-1>
```

- Default: `Auto`. Whisper-large-v3-turbo auto-detects per chunk; reasonable for monolingual sessions.
- Toggle button in the menu bar / record window: `Auto / EN / ES / IT / Other…`. State stored on the `Meeting`.
- On user click during recording: engine flushes the current window, rewinds 1.5 s, and starts the next window with the new `language_hint`. No model swap (Whisper is multilingual). Parakeet TDT v3 covers EN + 25 EU langs incl. ES; for IT or unsupported langs, engine downgrades to Whisper for that window.
- On post-pass: per-segment `language_detected` from live is honored; chunks with mismatched detection are flagged for review.
- Notes generation runs once with the dominant language as the LLM prompt language, but transcript is preserved verbatim per segment.

## Screen + audio capture

ScreenCaptureKit gives screen + system audio + per-app audio in one API on macOS 14+. macOS 26 Tahoe added HDR screenshot output; SCK itself unchanged. Per-app audio capture is the killer feature — record only the meeting app's audio, no music / system sounds bleeding in.

Inputs the user picks before starting:
- **Audio sources**: any combination of (default mic, default system audio, specific app audio, specific input device).
- **Video source**: full display, specific display, specific window, or none (audio-only mode).
- **Camera overlay**: optional, picture-in-picture, off by default.

Recording lifecycle:
- `Meeting::start(config)` → engine creates `~/.../meetings/<uuid>/` with `audio.m4a`, `video.mp4`, `meeting.db`, `screenshots/`.
- Mac shell pipes audio frames to engine for live ASR.
- Mac shell writes video directly to disk (never crosses IPC); video file path is registered in the meeting.
- Mac shell runs the screenshot sampler in-process: every 500 ms, downsamples the latest video frame to 320×180, computes Vision FeaturePrint, compares to last kept frame; if cosine distance > threshold (default 0.15), writes full-res JPEG to `screenshots/<timestamp_ms>.jpg` and notifies engine.
- `Meeting::stop()` → flush, finalize files, kick off post-pass.

## Screenshot-anchored notes

The differentiator. Pipeline:
1. Recording produces N timestamped screenshots.
2. Post-pass produces a transcript with start/end times per segment.
3. Notes generation prompt receives: the transcript + a list of `(timestamp, file_path, optional_caption)` screenshots + the user's notes template + the active `MarkdownDialect`'s syntax cheat-sheet.
4. LLM is instructed to embed screenshots inline when they're temporally relevant to the surrounding paragraph (`![](./screenshots/00:14:32.jpg)`) and to use dialect-appropriate syntax for callouts, toggles, action-item blocks, etc. Under `NotionEnhanced` it can emit `<callout icon="🎯">…</callout>`, `<details><summary>…</summary>…</details>`, `<table>` blocks, `{color="..."}` block colors. Under `Basic` it stays in CommonMark.
5. Default template carries verified-speaker rules and narrative reconstruction. User can swap templates; templates declare which dialect features they assume.
6. Output written by the active `NotesExporter`. The default `MarkdownExporter` produces `notes/YYYYMMDD-<slug>.md` with YAML frontmatter, the LLM-generated body, and relative image paths to `screenshots/`. Future `NotionExporter` POSTs the same dialect-`NotionEnhanced` body to the Notion API.

The strict speaker-verification rule becomes part of the prompt: "Never attribute a quote to a speaker unless segment metadata includes a confirmed `speaker_id` from diarization."

## IPC protocol

**Transport**: Unix domain socket at `~/Library/Application Support/panops/engine.sock`.
**Control plane**: JSON-RPC 2.0.
**Event plane**: WebSocket upgrade on the same socket (or a sibling socket); server pushes events.

Method shape (defined in `crates/panops-protocol`):

```
// Control
meeting.start(config: MeetingConfig) -> MeetingId
meeting.stop(id: MeetingId) -> Meeting
meeting.set_language(id, lang: Language) -> ()
meeting.list() -> [MeetingSummary]
meeting.get(id) -> Meeting
meeting.delete(id) -> ()

asr.post_pass(meeting_id, opts: PostPassOpts) -> JobId
asr.cancel(job_id) -> ()

notes.generate(meeting_id, template_id, llm_provider, dialect?) -> JobId
notes.export(meeting_id, exporter_id, opts) -> JobId      // markdown file | future: notion | obsidian | …

llm.probe() -> ProbeResult                                 // detects available providers + recommended default
llm.providers() -> [ProviderInfo]
llm.test(provider_id) -> TestResult

settings.get / settings.set
```

```
// Events (over WS)
{ "type": "asr.partial", "meeting_id": "...", "segment": Segment }
{ "type": "asr.final",   "meeting_id": "...", "segment": Segment }
{ "type": "screenshot",  "meeting_id": "...", "timestamp_ms": 873200, "path": "..." }
{ "type": "job.progress", "job_id": "...", "phase": "diarize", "pct": 0.42 }
{ "type": "job.done", "job_id": "...", "result": ... }
{ "type": "job.error", "job_id": "...", "error": "..." }
```

The sidecar (`panops-asr-mac`) speaks a sub-protocol on stdin/stdout — `json-lines`, one PCM-or-control message per line. Engine wraps it in the same `AsrProvider` trait the portable adapter implements.

## Storage model

Per-meeting SQLite at `~/.../meetings/<uuid>/meeting.db`:

```sql
CREATE TABLE meeting (
  id TEXT PRIMARY KEY, created_at INTEGER, started_at INTEGER, ended_at INTEGER,
  title TEXT, language TEXT, audio_path TEXT, video_path TEXT, config_json TEXT
);
CREATE TABLE segment (
  id INTEGER PRIMARY KEY, meeting_id TEXT, start_ms INTEGER, end_ms INTEGER,
  text TEXT, language TEXT, confidence REAL, speaker_id INTEGER,
  source TEXT  -- 'live' | 'post_pass'
);
CREATE TABLE speaker (
  id INTEGER PRIMARY KEY, meeting_id TEXT, label TEXT, embedding BLOB
);
CREATE TABLE screenshot (
  id INTEGER PRIMARY KEY, meeting_id TEXT, timestamp_ms INTEGER,
  path TEXT, feature_print BLOB, caption TEXT
);
CREATE TABLE note (
  id INTEGER PRIMARY KEY, meeting_id TEXT, template_id TEXT, content_md TEXT,
  dialect TEXT NOT NULL DEFAULT 'notion-enhanced',
  llm_provider TEXT, generated_at INTEGER
);
CREATE TABLE job (
  id TEXT PRIMARY KEY, meeting_id TEXT, kind TEXT, status TEXT,
  progress REAL, error TEXT, created_at INTEGER, finished_at INTEGER
);
```

Cross-meeting registry at `~/.../panops.db`: just `meeting_id → path` and global settings.

## Testing strategy

- `panops-core` is pure Rust: unit-tested directly.
- Each port has an `adapter-conformance` test suite in `tests/adapter-conformance/`. Both the `mac-native` and `portable` adapters run against the same suite. CI runs the portable suite on Linux + macOS; macOS-only job runs the mac-native suite too.
- Live ASR has a fixture set: short bilingual EN/ES clips with known transcripts. Adapters must hit a WER threshold; both must agree on segment count within tolerance.
- Diarizer: fixture meetings with known speaker counts; check ARI / DER thresholds.
- Notes generation: snapshot tests on prompt construction, not LLM output. LLM call is mocked in tests; real LLM calls are gated behind a manual integration test that costs API credits.
- IPC: integration tests spawn the engine, drive it via a test client, assert event sequences.
- Property-based tests (`proptest`) on segment merging / overlap deduplication / language-toggle state machine.

## Build & distribution

- Mac app: Xcode build → universal binary (arm64 + x86_64 fallback) → notarized `.dmg` and Homebrew cask.
- Engine binary: `cargo build --release` → bundled inside `.app` for the Mac distribution; standalone `.tar.gz` for headless / Linux / Windows.
- Sidecar binary: built with the Mac app's Xcode pipeline; not shipped to non-Mac targets.
- CI: GitHub Actions, matrix on (macOS-arm64, macOS-x86_64, ubuntu-latest, windows-latest), gating release on the conformance suite.
- Auto-update: Sparkle for the Mac app (standard OSS Mac app pattern).

## Decisions (locked 2026-04-30)

1. **License: MIT.** Maximum adoption, no licensing friction. Bundled libraries keep their own licenses (WhisperKit Apache-2.0, FluidAudio Apache-2.0, pyannote MIT, whisper.cpp MIT, etc.) — all compatible.
2. **Default LLM provider: machine-spec detection + first-run prompt with the detected default pre-selected.** On first launch the engine runs a `ProviderProbe`:
   - If on macOS 26+ → Apple FoundationModels available; pre-select it.
   - Else if `ollama` binary on `PATH` and `ollama list` returns at least one model → pre-select Ollama with the largest installed model.
   - Else if `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` / `GROQ_API_KEY` env vars present → pre-select the matching cloud provider.
   - Else → no default; user picks from the list.
   The first-run dialog shows the detected default highlighted ("Use Apple Intelligence (recommended for your Mac)") with a "Choose another" affordance. Detected default is a fallback the engine will use if the user dismisses the dialog without picking — it is never used silently the first time without consent.
3. **App display name = `panops`** (binary name, repo name, menu bar name — same everywhere).
4. **Notes format: enhanced markdown by default, basic markdown via toggle.** YAML frontmatter on every notes file.
   - **Frontmatter** (always emitted, both dialects):
     ```yaml
     ---
     title: <derived from first heading or LLM-generated>
     date: 2026-04-30
     started_at: 2026-04-30T14:32:11-03:00
     duration: 1h 23m 4s
     language: en           # or "mixed: en, es"
     participants: [Fran, Ada, ...]
     template: default
     dialect: notion-enhanced  # or "basic"
     panops_version: 0.1.0
     meeting_id: <uuid>
     audio: ./audio.m4a
     video: ./video.mp4
     screenshots_dir: ./screenshots/
     ---
     ```
   - **`MarkdownDialect` enum**: `NotionEnhanced` (default) | `Basic`. Future: `Obsidian`, `Roam`. Selectable globally in settings, overridable per meeting / per template.
   - **`NotionEnhanced` dialect** = Notion-flavored markdown ([Notion Enhanced Markdown reference](https://developers.notion.com/llms.txt)): supports `<callout>`, `<details>` toggles, `<columns>`, `<table>`, `<mention-*>`, `<table_of_contents>`, `{color="..."}` block colors, etc. Renders cleanly in Notion (POSTable to `/v1/pages` for the future Notion exporter), degrades gracefully in renderers that ignore unknown HTML-like tags.
   - **`Basic` dialect** = CommonMark-only fallback: headings, lists, tables, code blocks, image embeds, plain blockquotes. For Obsidian / GitHub / vanilla markdown viewers that don't tolerate the Notion extensions.
   - **`NotesGenerator` is dialect-aware**: the LLM prompt includes the active dialect's syntax cheat-sheet, so the model emits compliant markup for that target. Dialect is passed as a structured arg, not freeform.
   - **`NotesExporter` trait** ships at v1.0 with `MarkdownExporter` (writes the `.md` file + frontmatter + screenshots beside it). Future: `NotionExporter` (POSTs the Notion-Enhanced markdown to the Notion API), `ObsidianExporter` (path conventions), etc. Adding an exporter is a new adapter, no UI changes.
5. **Telemetry: zero, ever.** No phone-home, no opt-in toggle, no anonymous metrics. Crash logs stay local.

## Decision-driven additions to the architecture

- New port: `LlmProviderProbe` in `panops-core`. `mac-native` adapter detects macOS version + FoundationModels availability via the LLM sidecar. `portable` adapter checks `PATH` for `ollama`, env vars for cloud keys.
- New port: `NotesExporter` in `panops-core`. `MarkdownExporter` (default, ships at v1.0) writes `.md` + frontmatter + screenshots. Future exporters drop in as new adapters.
- New domain type: `MarkdownDialect` enum. Stored on `Meeting`, defaulted from global setting, overridable per-meeting in the UI.
- `notes` table: add `dialect TEXT NOT NULL DEFAULT 'notion-enhanced'` column. The `content_md` column already covers both dialects.

