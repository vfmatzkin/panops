# panops-asr-mac

WhisperKit-backed ASR sidecar for the panops engine. Slice 10. Spec: `docs/superpowers/specs/2026-05-12-slice-10-whisperkit-asr-sidecar-design.md`.

## Dev setup

```bash
# 1. Build the sidecar (downloads WhisperKit model on first run; ~30s for tiny, ~60s for base)
cd apps/panops-asr-mac
swift build --configuration release

# 2. Point the engine at it
export PANOPS_ASR_SIDECAR_BIN="$PWD/.build/release/panops-asr-mac"

# 3. Run the engine — it picks the sidecar over whisper-rs on macOS
cd ../..
cargo run --release -p panops-engine -- notes <audio.wav>
```

The sidecar is a single-tenant child process. The engine spawns it on the first ASR call, keeps it alive across calls (model load is amortized), and SIGTERMs it on engine shutdown. Stdio carries newline-delimited JSON-RPC: one request line in, one response line out.

## Model selection

Default: `openai_whisper-base` (~150 MB, multilingual). Override:

```bash
PANOPS_WHISPERKIT_MODEL=openai_whisper-small <command>
```

The sidecar calls `WhisperKit.fetchAvailableModels(from: "argmaxinc/whisperkit-coreml")` on startup to validate the requested variant against the upstream HF repo; falls back to the literal string if the list fetch fails (offline path).

Available variants (verified 2026-05-12): `openai_whisper-tiny`, `openai_whisper-tiny.en`, `openai_whisper-base`, `openai_whisper-base.en`, `openai_whisper-small`, `openai_whisper-small.en`, plus distil + large variants.

## Conformance

```bash
PANOPS_ASR_SIDECAR_BIN="$PWD/apps/panops-asr-mac/.build/release/panops-asr-mac" \
cargo test --release --locked -p panops-mac --test conformance_whisperkit
```

Slice 10 ships with English-only conformance because `openai_whisper-base` auto-detects Spanish audio as English (and translates it to English text — see issue #125). Spanish + bilingual fixtures are skipped pending resolution.

## Requirements

- macOS 14.0+
- Xcode 16.0+ (Command Line Tools alone is NOT enough — WhisperKit needs Xcode's CoreML toolchain).
- Disk: ~40 MB (tiny) – ~150 MB (base) – ~1 GB (large variants). WhisperKit caches under HuggingFace Hub conventions (likely `~/Documents/huggingface/` for the default downloader).

## Architecture

```
panops-engine (Rust)
    ↓ spawn + JSON-RPC over stdio
panops-asr-mac (Swift)
    ↓ WhisperKit + CoreML/Metal
```

Wire format (per request):

```json
{"jsonrpc":"2.0","id":1,"method":"asr.transcribe","params":[{"audio":"/tmp/x.wav","sample_rate":16000,"language_hint":null}]}
```

Response:

```json
{"jsonrpc":"2.0","id":1,"result":{"schema_version":2,"model":"whisperkit-openai_whisper-base","audio_path":"...","audio_duration_ms":26160,"diarized":false,"segments":[...]}}
```

The `WireSegment` shape mirrors `panops_core::Segment`: `start_ms`, `end_ms`, `text`, `language_detected`, `confidence`, `is_partial`, `speaker_id`. Times are in milliseconds (converted from WhisperKit's float-seconds at the adapter boundary). Whisper special tokens (`<|startoftranscript|><|en|><|transcribe|><|0.00|>` etc.) are stripped from `text` via a regex.

## Known limitations

See open `area:asr` debt issues:

- **#125**: WhisperKit base auto-detects Spanish audio as English; conformance skips Spanish/mixed fixtures.
- (Future) Streaming partial events — Anchor B.
- (Future) Pre-bundle sidecar in `.app/Contents/Resources/panops-asr-mac` — slice 12 (sign + notarize).
