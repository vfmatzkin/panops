---
date: 2026-04-30
topic: Local macOS recorder + transcriber + notes app — market scan & existing-asset map
status: research notes (pre-design)
---

# Research notes — local recorder + transcriber + notes

## TL;DR
The wedge nobody owns: **open-source, local-first, audio + screen + screenshot-anchored meeting notes**. Tier-1 paid apps (MacWhisper, TranscribeX, BB Recorder, Superwhisper) are audio-only. OSS priors (Meetily, Hyprnote) are audio-only. Screenpipe captures screen+audio but is a passive search index, not a notes generator. Granola/Otter are cloud and don't store media. The "screen recording with auto-sampled screenshots tied to transcript timestamps, summarized into a notes doc with embedded frames" angle is unclaimed.

## Pain points to remove (from current manual workflow)

1. OBS setup (pick window, start, stop) — should be a one-click button.
2. Pre-record then upload — should be live.
3. Manual ffmpeg screenshot extraction — should be auto-sampled on visual change.
4. Separate AI session run by hand — should be in-app, BYO key.
5. Multi-step app-switching — should be one app from "record" to "notes".

## Competitive map

### Tier 1 — paid native Mac, audio-focused
| App | Recording | Live | Local engine | API engine | Diariz | Notes | Screenshots | Notable |
|---|---|---|---|---|---|---|---|---|
| MacWhisper | mic + sys audio | yes | Whisper Tiny→v4, **Parakeet v2/v3 MLX (~300x RT M-series)** | ElevenLabs/Deepgram | yes (Pro) | yes | no | speed king |
| TranscribeX | mic + sys audio | yes | Whisper, Parakeet | OpenAI/Gemini/Ollama | yes (Pro) | yes | no | YouTube bulk |
| BB Recorder | mic + sys audio | yes | bundled Whisper | BYO Deepgram/OAI/Anthropic | yes | yes | **no (audio only — verified)** | $0 forever, BYO keys |
| Superwhisper | mic-first dictation | yes | Whisper Large | OpenAI/Claude/Llama/etc | ? | "Meeting Assistant" | no | dictation-anywhere |
| Aiko | file only | no | Whisper-v3 | none | no | none | no | free, accuracy |

### Tier 2 — meeting-notes (cloud)
- **Granola** ($14–35) — bot-free system audio, "Enhance Notes" on user bullets, doesn't store media. Cloud-only.
- **Cleft** ($7–9) — voice-only, on-device transcript, neurodivergent angle.
- **Otter** ($8–20) — desktop bot-free or bot-on-call, agents.
- **Wispr Flow** — dictation only, cloud.

### Tier 3 — OSS building blocks
- **whisper.cpp** — Metal/Neural Engine, GGML quantized. The de facto local Whisper.
- **mlx-whisper / mlx-audio (Parakeet MLX)** — fastest on Apple Silicon. Parakeet ~0.5s vs whisper-turbo ~1.0s same clip.
- **WhisperKit** — Argmax's Apple-optimized Whisper, used by MacWhisper.
- **WhisperX** — wav2vec2 forced alignment, ±50ms word timestamps + diarization, 70x RT large-v2.
- **pyannote.audio Community-1** — open diarization standard.
- **Parakeet TDT v2/v3 (NVIDIA)** — fastest single-pass ASR, English-heavy, 25 EU langs.
- **whisper-large-v3-turbo** — 809M, ~1–2% WER off large-v3, 99 langs.
- **distil-whisper** — 6x faster, English-only.
- **Meetily** (MIT, Rust) — closest prior art: Parakeet/Whisper + Ollama + BYO Claude/Groq/OpenRouter. Audio-only.
- **Hyprnote / Char** — OSS, markdown-native, sys audio, BYO LLM. Audio-only.
- **Screenpipe** (MIT) — 24/7 screen+audio capture with OCR + transcript search. Search index, not notes.

## Niche analysis

1. **Screenshots-anchored local notes is the real gap.** No Tier 1 / Tier 2 / OSS competitor ships it. BBRecorder's marketing positions it as audio-only despite the screenshot the user saw — likely a non-shipped or misread feature. Wide open.
2. **Live-transcription stack 2026** — leaders run Parakeet (MLX) for speed-tier + Whisper-large-v3-turbo (WhisperKit/MLX) for accuracy-tier + pyannote for diarization. Parakeet hits 300x RT on M-series; turbo is the multilingual default.
3. **Defensible angles for a one-person OSS project (pick two, max)**:
   - **A) Notes-with-screenshots**: screen + sys audio + mic → live Parakeet → screenshots sampled on slide/window change (CGWindowList + perceptual-hash diff) → LLM emits markdown with embedded frames. Drop-in for Obsidian.
   - **B) BYO-everything**: local Whisper/Parakeet + optional pyannote + BYO Claude/OpenAI/Ollama for summary. Replaces the manual OBS → batch-transcript → manual-notes chain.
   - Skip: another dictation app (saturated), cloud meetings (Granola/Otter own it), passive memory (Screenpipe).

## Constraints / decisions to make
- **Form factor**: native Swift app vs Tauri/Electron vs Python web app. Determines screen-capture API access, distribution path, dev velocity.
- **Live engine**: Parakeet MLX (fast, mostly English) vs Whisper-turbo via WhisperKit (multilingual) vs both with auto-pick.
- **Screen capture**: ScreenCaptureKit (macOS 12.3+, AVFoundation-tied) is the only modern path; gives system-audio + screen in one shot.
- **Diarization**: bundle pyannote (heavy, ~1GB) vs optional download vs API-only.
- **Output target**: Markdown + Obsidian links by default; SRT/VTT for transcript-only export.
- **Scope of MVP**: audio-first then screen, screen-from-day-1, or full-bore.

## Open questions for design phase
1. MVP scope: audio+notes only (replace existing chain) vs full screen+screenshots from v0.1?
2. Stack: Tauri (Rust+web UI, easiest cross-target later) vs SwiftUI native (best ScreenCaptureKit story, Mac-only)?
3. Live engine default: Parakeet or Whisper-turbo? (auto-detect by language?)
4. Multi-language? Fran works in English/Spanish/Italian — Parakeet is English-heavy.
5. Distribution: GitHub release dmg, Homebrew cask, or Mac App Store?
6. Should it record meetings end-to-end OR also do "anywhere dictation" (Superwhisper space)? Probably no — stay focused.
