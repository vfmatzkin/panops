# Panops Mac shell

SwiftUI walking-skeleton client for the `panops-engine` IPC. Slice 09. See `docs/superpowers/specs/2026-05-11-slice-09-mac-shell-walking-skeleton-design.md` for the spec.

## Dev setup

```bash
# 1. Build the Rust engine.
cd /Users/fran/Code/panops
cargo build --release -p panops-engine

# 2. Tell the Mac shell where to find it.
export PANOPS_ENGINE_BIN="$PWD/target/release/panops-engine"

# 3. Build + run the Swift app.
cd apps/Panops
swift run Panops
```

`PANOPS_ENGINE_BIN` is the dev escape hatch (spec D3). Production builds will find the engine via `Bundle.main.bundleURL/Contents/Resources/panops-engine` once slice 12 ships the `.app` packaging.

## Tests

```bash
cd apps/Panops
swift test
```

Four IPC codec round-trip tests. No view-model or UI tests this slice.

## Manual smoke

Use the public test fixture, not private recordings:

```bash
swift run Panops
# In the app: Open audio… → crates/panops-engine/tests/fixtures/audio/en_30s.wav
# → Generate notes → wait → Done with the notes path → Open in Finder
```

Verify the engine SIGTERMs on quit:

```bash
pgrep panops-engine  # should be empty within 5s of Cmd+Q
```
