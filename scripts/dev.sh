#!/usr/bin/env bash
# Quick dev launch (no signing) — for UI iteration.
# Builds the engine + capture sidecar (debug), then runs the app from source.
#
# NOTE: live Screen Recording needs the SIGNED bundle (scripts/package.sh) —
# a bare `swift run` binary can't reliably get the macOS Screen Recording
# permission. Use this for browsing the UI / generating notes from an existing
# audio file; use package.sh to test the full record→transcribe→notes flow.
#
# Builds with the Command Line Tools (no full Xcode needed) — the engine, the
# app, and the capture sidecar all build under CLT. (The ASR + LLM sidecars are
# NOT built here; the engine falls back to whisper-rs + local ollama.)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
REPO="$(pwd)"

cargo build -p panops-engine
( cd apps/panops-capture-mac && swift build )

cd apps/Panops
PANOPS_ENGINE_BIN="$REPO/target/debug/panops-engine" \
PANOPS_CAPTURE_SIDECAR_BIN="$REPO/apps/panops-capture-mac/.build/debug/panops-capture-mac" \
exec swift run
