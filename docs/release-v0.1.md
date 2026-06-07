# v0.1 Release Checklist

Prerequisites: merged PR for this slice, clean working tree on `main`.

## 1. Build + ad-hoc-sign Panops.app

```bash
git checkout main
git pull
scripts/package.sh v0.1.0
```

Output: `dist/Panops-v0.1.0.tar.gz` + `dist/Panops-v0.1.0.tar.gz.sha256`.

## 2. Compute tarball sha256

```bash
cat dist/Panops-v0.1.0.tar.gz.sha256
```

Copy this hex digest for the cask update.

## 3. Create models-v1 release + upload model assets

Create a GitHub release tagged `models-v1` with all model files as assets.

### Model assets (with sha256 from `crates/panops-portable/src/model.rs`)

| Asset | sha256 |
|---|---|
| `ggml-tiny-q5_1.bin` | `818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7` |
| `ggml-base-q5_1.bin` | `422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898` |
| `ggml-large-v3-turbo-q5_0.bin` | `394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2` |
| `sherpa-onnx-pyannote-segmentation-3-0.tar.bz2` | `24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488` |
| `3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx` | `1a331345f04805badbb495c775a6ddffcdd1a732567d5ec8b3d5749e3c7a5e4b` |
| `ggml-silero-v6.2.0.bin` | `2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987` |

```bash
# Upload each asset to the models-v1 release
gh release create models-v1 \
  --repo vfmatzkin/panops \
  --title "Model assets for v0.1" \
  --notes "Model files for Panops v0.1. Mirror URLs in panops-portable use this release as primary, upstream as fallback." \
  path/to/ggml-tiny-q5_1.bin \
  path/to/ggml-base-q5_1.bin \
  path/to/ggml-large-v3-turbo-q5_0.bin \
  path/to/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2 \
  path/to/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx \
  path/to/ggml-silero-v6.2.0.bin
```

Model files must already exist locally (from CI cache or manual download). Obtain from upstream URLs documented in `model.rs` if needed.

## 4. Update Casks/panops.rb

```ruby
cask "panops" do
  version "0.1.0"
  sha256 "<TARBALL_SHA256_FROM_STEP_2>"  # e.g. "a1b2c3..."

  url "https://github.com/vfmatzkin/panops/releases/download/v#{version}/Panops-#{version}.tar.gz"
  # ... rest unchanged
end
```

Commit: `update cask for v0.1.0 release`

## 5. Tag v0.1.0

```bash
git tag -a v0.1.0 -m "v0.1.0 release"
git push origin v0.1.0
```

## 6. Create GitHub Release v0.1.0

```bash
gh release create v0.1.0 \
  --repo vfmatzkin/panops \
  --title "v0.1.0" \
  --notes-file RELEASE_NOTES.md \
  dist/Panops-v0.1.0.tar.gz \
  dist/Panops-v0.1.0.tar.gz.sha256
```

### Release notes draft (v0.1.0)

First public release of Panops — a local-first macOS recorder with screenshot-anchored meeting notes.

Features:

- Headless CLI for transcription + diarization + note generation
- SwiftUI Mac app with WhisperKit ASR sidecar
- SQLite per-meeting storage + cross-meeting registry
- JSON-RPC + WebSocket IPC over Unix domain socket
- VAD-aware multilingual ASR (per-region language detection)
- Markdown notes output (NotionEnhanced dialect)

Known limitations:

- Ad-hoc signed (not Apple-notarized). Requires quarantine flag bypass on first launch.
- No live capture yet (planned for post-v0.1).

Install via Homebrew:

```bash
brew tap vfmatzkin/panops
brew install --cask panops
```

## 7. Verify brew install

```bash
brew install --cask panops
xattr -dr com.apple.quarantine "$(brew --prefix)/Caskroom/panops/0.1.0/Panops.app"
open "$(brew --prefix)/Caskroom/panops/0.1.0/Panops.app"
```

Confirm app launches, UI renders, sidecars start.

---

Post-release: close v0.1 milestone on project board, run slice-boundary alignment audit per AGENTS.md.