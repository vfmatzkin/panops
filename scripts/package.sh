#!/usr/bin/env bash
# Build + assemble + ad-hoc-sign Panops.app, then tar it. No Apple account.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# Preflight: the ASR (WhisperKit/CoreML) + LLM (FoundationModels) sidecars need
# FULL Xcode, not just the Command Line Tools.
if [[ "$(xcode-select -p 2>/dev/null)" == *CommandLineTools* ]]; then
  echo "error: full Xcode required (xcode-select points at CommandLineTools)." >&2
  echo "  Install Xcode, then: sudo xcode-select -s /Applications/Xcode.app" >&2
  exit 1
fi

VERSION="${1:-$(git describe --tags --always)}"
OUT="dist"; APP="$OUT/Panops.app"; RES="$APP/Contents/Resources"; MACOS="$APP/Contents/MacOS"
rm -rf "$OUT"; mkdir -p "$RES" "$MACOS"

# 1. Release builds (engine + app + all three Swift sidecars: ASR, LLM, capture)
cargo build --release -p panops-engine
( cd apps/Panops            && swift build -c release )
( cd apps/panops-asr-mac    && swift build -c release )
( cd apps/panops-llm-mac    && swift build -c release )
( cd apps/panops-capture-mac && swift build -c release )

# 2. Assemble. Sidecars sit in Resources/ next to panops-engine so the engine's
# sibling-of-engine resolver finds them (asr_resolver / llm_resolver /
# capture_resolver) — no env vars needed in the bundle.
cp target/release/panops-engine "$RES/panops-engine"
cp apps/Panops/.build/release/Panops "$MACOS/Panops"
cp apps/panops-asr-mac/.build/release/panops-asr-mac "$RES/panops-asr-mac"
cp apps/panops-llm-mac/.build/release/panops-llm-mac "$RES/panops-llm-mac"
cp apps/panops-capture-mac/.build/release/panops-capture-mac "$RES/panops-capture-mac"

# 3. Info.plist
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Panops</string>
  <key>CFBundleIdentifier</key><string>dev.panops.Panops</string>
  <key>CFBundleExecutable</key><string>Panops</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>NSMicrophoneUsageDescription</key><string>Panops records meeting audio to transcribe it on-device.</string>
</dict></plist>
PLIST

# 4. Ad-hoc sign (inner binaries first, then the app), hardened runtime + entitlements
for b in "$RES/panops-engine" "$RES/panops-asr-mac" "$RES/panops-llm-mac" "$RES/panops-capture-mac"; do
  codesign --force --options runtime --timestamp=none -s - "$b"
done
codesign --force --options runtime --timestamp=none \
  --entitlements apps/Panops/Panops.entitlements -s - "$APP"
codesign --verify --deep --strict "$APP"

# 5. Tar + sha256 (bare hash in the .sha256 file — the cask needs just the
# digest; the human-readable line goes to stdout).
TARBALL="$OUT/Panops-${VERSION}.tar.gz"
tar -C "$OUT" -czf "$TARBALL" Panops.app
shasum -a 256 "$TARBALL" | awk '{print $1}' > "$TARBALL.sha256"
echo "built $TARBALL  sha256=$(cat "$TARBALL.sha256")"

cat <<DONE

✓ Signed app: $APP
  To test the full record → transcribe → notes flow with permissions:
    open $APP
  Then grant Screen Recording + Microphone in
  System Settings → Privacy & Security (relaunch the app after granting).
  (Requires macOS 26 + Apple Intelligence enabled for on-device notes.)
DONE
