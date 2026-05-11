# Slice 09 — Mac Shell Walking Skeleton

**Status:** Locked design. Open for plan-writing.
**Date:** 2026-05-11
**Author:** Franco Matzkin (with Claude as brainstorm partner)
**Predecessor:** [slice 08 design](2026-05-11-slice-08-confidence-recursion-design.md)
**North-star tie-in:** First step of Anchor A. Makes panops *usable as an app* — v0.1 acceptance criterion #1 ("open the Mac app, hit record") is unmet until this lands. Closes the perf-concern → Anchor A path by laying the ground for the WhisperKit ASR sidecar (next slice).

## Problem

After slice 08, the multilingual-day-1 north-star promise holds end-to-end — but only for the headless `panops-engine` CLI. There is no Mac app. v0.1 acceptance criteria #1–3 ("open the app, hit record / stop / generate notes") are mechanically blocked: there's nothing to open. AGENTS.md anchors `apps/Panops/` as the SwiftUI app, with sidecars at `apps/panops-asr-mac/` and `apps/panops-llm-mac/`. None of that exists yet.

The maintainer's perf concern raised mid-slice-08 — current whisper-rs on CPU is 5–10× slower than necessary on Apple Silicon — needs the WhisperKit-based macOS ASR sidecar to land. That sidecar is the next slice; this slice is the foundation it plugs into.

## Goal

Ship the thinnest possible end-to-end SwiftUI app: launches a window, spawns the existing `panops-engine` as a child process, lets the user pick an audio file, calls `notes.generate` over the existing UDS, watches the WebSocket for `job.done`, displays the resulting notes path. Three states: Idle, Working, Done/Error. No new Rust code. No sidecars yet. No live capture. No code signing. The slice exists to prove the IPC contract works from a real Mac app — every subsequent Anchor-A slice composes on top.

## Decisions

| # | Decision | Reason |
|---|---|---|
| D1 | **Build system: SwiftPM** at `apps/Panops/Package.swift`. Not Xcode `.xcodeproj` | Text-only manifest, reviewable diffs, no binary project files; `swift build` runs from CI without an Xcode workspace; SwiftUI apps build under SwiftPM since Swift 5.5. Code signing at slice 12 uses `codesign` CLI, not Xcode-only paths. |
| D2 | **App owns the engine process lifecycle.** App launches → spawns `panops-engine serve` as a child; app quits → engine receives SIGTERM | Natural Mac-app UX (criterion #1 doesn't say "launch Terminal first"). Matches the path packaging will take at slice 12 (engine binary bundled in `.app/Contents/Resources/`). |
| D3 | **Engine binary location: `PANOPS_ENGINE_BIN` env var (dev escape hatch) with `Bundle.main` fallback for production** | AGENTS.md "no env vars for user config" rule explicitly allows dev/CI escape hatches when flagged. Production lookup is `Bundle.main.bundleURL/Contents/Resources/panops-engine`. The env var path documented in `apps/Panops/README.md` and in this spec. |
| D4 | **Socket path: default only**, no override flag in slice 09 | The engine's `--socket` flag already exists; not exposing it in the Swift UI keeps the slice tiny. If smoke later shows two app instances conflicting, expose then. |
| D5 | **Hand-rolled JSON-RPC + WebSocket clients** built on `Foundation.URLSession` and `NWConnection` (`Network` framework) — no external SwiftPM dependencies this slice | Single-app dependency surface. Adding `starscream` or similar pulls in CocoaPods-era machinery; `NWConnection` is the platform-native answer. |
| D6 | **Audio format: WAV/M4A/MP3/MOV accepted by the `NSOpenPanel` UTType filter**; engine handles content validation | The engine's `load_wav_mono16k` only accepts 16-kHz mono WAV today, but the file picker shouldn't pre-filter; bad inputs surface as `job.error` with `input_not_found` / `invalid_input` kinds. Slice 10 (ASR sidecar) likely broadens accepted formats. |
| D7 | **UI states are three: `Idle`, `Working(job_id)`, `Done(path)` / `Error(kind, message)`** | Three-state UI is small enough to fit on one screen and exhaustively testable. No progress bar (no `job.progress` events ship today — `docs/proto/ipc.md:128` explicitly defers them). |
| D8 | **No SwiftUI snapshot tests; one `IpcClient` JSON unit test; one manual smoke** | UI snapshots are overkill for one window. JSON encoding/decoding is pure logic and worth testing. The manual smoke covers the integration that XCTest can't reliably hit. |

## Scope

### In

1. **New top-level directory** `apps/Panops/` containing:
   - `Package.swift` (SwiftPM manifest, Swift 5.9+, macOS 14+).
   - `Sources/Panops/PanopsApp.swift` — SwiftUI `@main` app entry.
   - `Sources/Panops/EngineProcess.swift` — child-process spawn/teardown.
   - `Sources/Panops/IpcClient.swift` — JSON-RPC + WS client actor.
   - `Sources/Panops/ContentView.swift` — single-window UI.
   - `Sources/Panops/Models.swift` — Codable structs mirroring `panops-protocol`'s wire types.
   - `Tests/PanopsTests/IpcClientCodecTests.swift` — JSON round-trip unit test.
   - `README.md` — developer setup (env var for engine binary, etc.).
2. **`.gitignore` entries** for `apps/Panops/.build/` and `apps/Panops/.swiftpm/`.
3. **CI**: extend `.github/workflows/ci.yml` `test (macos-latest)` job (or add a sibling job) to run `cd apps/Panops && swift build` on the macOS runner.
4. **No Rust changes.** Verify by `git diff --stat main..HEAD` showing zero changes outside `apps/Panops/`, `.github/`, and `.gitignore`.

### Out (filed as debt if surfaced during implementation)

- **Meetings list** UI (uses existing `meeting.list`). Slice 11 candidate, after sidecars settle.
- **Per-meeting language toggle** UI (north-star requirement). Slice 11+ candidate.
- **In-app markdown rendering**. Open-in-Finder is enough this slice.
- **WhisperKit / FluidAudio ASR sidecar**. Next slice (the perf fix).
- **FoundationModels LLM sidecar**. Slice after that.
- **Live capture (Anchor B)**. Risk-last surface, last anchor.
- **Code signing, notarization, `.app` bundle scripts**. v0.1 acceptance #6, separate slice.
- **Reconnect logic** if the engine dies mid-job. Engine death = app dies (D2 contract).
- **Multiple-instance protection** (two apps racing for the same socket). Engine's existing `ipc_refuses_to_steal_live_socket` test covers the engine side; Swift app surfaces the error.
- **Settings pane, About box, menu bar**. SwiftUI's default `.commands { }` is enough; no custom menus.
- **Crash reporting, telemetry**. North-star: zero telemetry, ever.
- **Stale-socket cleanup on app start** (left from a previous engine that didn't shut down cleanly). Engine's existing `stale_socket_is_unlinked_and_rebound` test handles this engine-side.

## Architecture

```
┌─────────────────────────────────┐
│  apps/Panops/   (SwiftUI app)   │
│  ┌───────────────────────────┐  │
│  │ ContentView (3 states)    │  │
│  └─────────┬─────────────────┘  │
│            │                    │
│  ┌─────────▼─────────────────┐  │
│  │ IpcClient (actor)         │──┼──┐ JSON-RPC + WS over UDS
│  │   - notesGenerate(path)   │  │  │
│  │   - subscribe(events)     │  │  │
│  └───────────────────────────┘  │  │
│                                 │  │
│  ┌───────────────────────────┐  │  │ spawns + SIGTERMs
│  │ EngineProcess (struct)    │──┼──┼───────────┐
│  │   - start() / stop()      │  │  │           │
│  └───────────────────────────┘  │  │           │
└─────────────────────────────────┘  │           │
                                     │           │
              ┌──────────────────────▼───────────▼──┐
              │  panops-engine serve  (existing)   │
              │  ~/Library/Application Support/    │
              │    panops/engine.sock              │
              └────────────────────────────────────┘
```

The Swift app is a pure IPC client. The engine is unchanged. The contract surface — `ipc.notes.generate`, `ipc.events.subscribe`, `job.done` / `job.error` events — is exactly what `docs/proto/ipc.md` already documents.

## Components (Swift side)

### `EngineProcess`

```swift
struct EngineProcess {
    private let process: Process
    static func start(binary: URL, socket: URL? = nil) throws -> EngineProcess
    func stop() async   // sends SIGTERM, waits up to 5s, then SIGKILL
    var isRunning: Bool { get }
}
```

- Resolves the engine binary path: `ProcessInfo.processInfo.environment["PANOPS_ENGINE_BIN"]` first, then `Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/panops-engine")`.
- Passes `serve` as the first arg. No `--socket` override (D4).
- Pipes engine stderr to the app's stderr so engine logs surface in the same Console.app stream (dev *and* production).
- `deinit` invokes `stop()` synchronously as a last-ditch cleanup.

### `IpcClient`

```swift
actor IpcClient {
    init(socketPath: URL) async throws
    func notesGenerate(audio: URL, dialect: String?, language: String?, ...) async throws -> String  // job_id
    func subscribeEvents() -> AsyncStream<IpcEvent>
    func disconnect() async
}

enum IpcEvent {
    case jobDone(jobId: String, result: JobDoneResult)
    case jobError(jobId: String, kind: String, message: String)
    case unknown  // forward-compatible; skip
}
```

- Opens a `NWConnection` to the UDS path; uses two logical streams over it: JSON-RPC requests/responses framed as newline-delimited JSON; WebSocket events on the upgrade path.
- *Implementation note*: the existing engine accepts the WS upgrade on the same UDS; the client speaks RFC 6455 over the connection. If `NWProtocolWebSocket` proves painful, fall back to a tiny hand-rolled frame parser (the protocol surface needed is text-frame-only, no binary, no extensions).
- Maps unknown event `type` values to `.unknown` rather than throwing — matches the IPC contract (line 128 of `docs/proto/ipc.md`).

### `ContentView`

State machine:

```
              click "Open audio..."
   ┌─Idle─────────────────────────┐
   │  audio: URL? = nil           │  ◄────────────────────┐
   └──────┬───────────────────────┘                       │
          │ user picked file                              │
          ▼                                               │
   ┌─Idle (audio set)─────────────┐                       │
   │  audio: URL (non-nil)        │                       │
   └──────┬───────────────────────┘                       │
          │ click "Generate notes"                        │
          ▼                                               │
   ┌─Working(job_id)──────────────┐                       │
   │  spinner + "Working..."      │                       │
   └──────┬────────────┬──────────┘                       │
          │            │                                  │
   job.done│           │job.error                         │
          ▼            ▼                                  │
   ┌─Done(path)─┐   ┌─Error(kind, msg)─┐  click "Retry"   │
   │ Open in    │   │ Show kind/msg    │──────────────────┘
   │ Finder     │   │ Retry button     │
   └────────────┘   └──────────────────┘
```

UI elements: title text, two buttons (Open / Generate), a status text, a conditional "Open in Finder" button. No tabs, no sheets, no menus beyond SwiftUI's defaults.

### `Models`

Plain `Codable` structs for the wire types we use:

```swift
struct NotesGenerateParams: Encodable {
    let audio: String
    let dialect: String?
    let language: String?
    let meetingId: String?
    // ... mirror panops-protocol's NotesGenerateParams
}

struct NotesGenerateResult: Decodable {
    let jobId: String
}

struct JobDoneResult: Decodable {
    let primaryFile: String
    let assets: [String]
    let meetingId: String
}
```

Field names use Swift camelCase with `CodingKeys` mapping to the snake_case wire format.

## Data flow (happy path)

1. **App launch**: `PanopsApp.init()` → `EngineProcess.start(binary: ...)`. Engine starts; UDS file appears after a moment.
2. **Connect**: `Task { let client = try await IpcClient(socketPath: ...) }`. Retries with exponential backoff up to 5s if the socket isn't ready (engine cold-start).
3. **Subscribe**: `client.subscribeEvents()` — receives events asynchronously into the view model.
4. **User opens file**: `NSOpenPanel` configured with `UTType.wav`, `UTType.mpeg4Audio`, `UTType.mp3`, `UTType.movie`. Result captured in `audio: URL`.
5. **User clicks Generate**: `client.notesGenerate(audio: ...)` → `{job_id}`. ViewModel transitions to `Working(job_id)`.
6. **Event arrives**: matching `job.done` → `Done(primaryFile)`. Non-matching `job_id` (multiple concurrent jobs, future-proof) ignored.
7. **User clicks Open in Finder**: `NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: primaryFile)])`.
8. **App quit**: SwiftUI lifecycle hook → `await client.disconnect()` → `await engineProcess.stop()`. Engine has 5s to drain via SIGTERM; if not gone, SIGKILL.

## Error paths

| Scenario | Behavior |
|---|---|
| Engine binary not found at startup | Fatal alert: "Could not find panops-engine binary. Set PANOPS_ENGINE_BIN to the absolute path (dev) or rebuild the .app (prod)." Quit on dismiss. |
| Engine crashed within 5s of launch | Alert with engine stderr's last 1KB. Quit on dismiss. |
| Socket file never appears | Same alert as crash, message: "Engine did not bind socket within 5s." |
| `notes.generate` returns RPC error | Surface error.message in the UI; revert to Idle (audio kept selected). |
| WS connection drops mid-job | Reconnect once; if reconnect fails, surface "Lost connection to engine. Quit and relaunch." No automatic retry of the in-flight job (state is on the engine side via the meeting registry). |
| User quits during Working | `engineProcess.stop()` SIGTERMs. The in-flight pipeline will receive cancellation upstream (existing `cancelled` job-error kind covers this). |

## Testing

### Unit (Swift)

`Tests/PanopsTests/IpcClientCodecTests.swift`:

1. `notesGenerateRequest_encodesParams` — round-trips `NotesGenerateParams { audio: "/tmp/x.wav" }` → JSON → wire-format check (snake_case field names, no extras).
2. `jobDoneEvent_decodes` — parses a known-good `{ "type": "job.done", "job_id": "abc", "result": { "primary_file": "...", ... } }` into `IpcEvent.jobDone(...)`.
3. `jobErrorEvent_decodes` — same for `job.error` with each of the 5 documented `kind` values.
4. `unknownEventType_doesNotThrow` — `{"type": "asr.partial", ...}` parses to `IpcEvent.unknown` (forward-compat).

No view model tests; the state machine is exercised by the manual smoke.

### Manual smoke (pre-PR; captured in session log, NOT committed publicly)

From repo root:

```bash
cargo build --release -p panops-engine
export PANOPS_ENGINE_BIN="$PWD/target/release/panops-engine"
cd apps/Panops
swift run Panops
```

App launches → engine spawns (verifiable via `lsof | grep engine.sock`). Click Open → pick a known short test WAV (e.g., `crates/panops-engine/tests/fixtures/audio/en_30s.wav` — public test fixture, no private content). Click Generate → status goes to Working → after a few seconds → Done with the notes path. Click "Open in Finder" → Finder window shows the notes file. Quit the app → engine SIGTERM verifiable via `pgrep panops-engine` returning empty.

Pass criteria documented in the session log:

- Engine PID before app quit: `<N>`.
- Engine PID after app quit (within 5s): empty.
- Notes file `<path>` exists and is non-empty.

Do not run the smoke against the maintainer's private recordings; the public test fixture is sufficient.

### CI (automated)

Extend `.github/workflows/ci.yml`:

```yaml
- name: Build Swift Mac shell
  if: matrix.os == 'macos-latest'
  shell: bash
  run: |
    cd apps/Panops
    swift build --configuration release
```

Lands as a new step inside the existing `test (macos-latest)` job (or sibling job — implementation decides). Build-only; no run on CI (CI runners can't reliably exercise GUI). The unit test suite runs via `swift test`.

## Three-tier boundaries

### ✅ Always do

- `swift build` + `swift test` clean on macOS 14+ before each commit on the Swift side.
- `cargo fmt && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` still green on the Rust side (zero Rust changes expected, but verify).
- Commit per task in the slice plan.
- File a GitHub issue for any "deferred" / "out of scope" item per the Debt Rule.
- Use `Bundle.main.bundleURL` (not a hardcoded path) for the production engine lookup.

### ⚠️ Ask first

- Adding a SwiftPM dependency beyond `Foundation` / `SwiftUI` / `Network`.
- Adding any SwiftPM target beyond `Panops` (the app) and `PanopsTests`.
- Changing the engine spawn contract (e.g., passing extra flags, changing the socket path).
- Touching the Rust workspace at all this slice.
- Introducing a second window, sheet, or modal.
- Adding `.entitlements`, codesigning, or notarization — those are slice 12.
- Renaming the `apps/Panops/` directory or restructuring its layout.

### 🚫 Never do

- Bundle WhisperKit, FluidAudio, FoundationModels, or any ML framework this slice — they're the *next* slices.
- Add a Settings pane, preferences UI, or any persistent Swift-side state.
- Add an Xcode `.xcodeproj` alongside the SwiftPM manifest. One build system.
- Read or write to `panops.db` from Swift. The engine owns storage.
- Phone home. Zero telemetry, ever.
- Use `os_log` with public messages that include user content. Engine paths are fine; transcript content is not (and the shell never receives transcript content this slice).
- Auto-merge the PR.

## Acceptance criteria

1. `cd apps/Panops && swift build --configuration release` succeeds on macOS 14+.
2. `cd apps/Panops && swift test` succeeds — 4 IPC codec tests pass.
3. With `PANOPS_ENGINE_BIN` set, `swift run Panops` opens a window.
4. App spawns a `panops-engine serve` child process within 5 seconds of launch (verifiable via `pgrep panops-engine`).
5. Manual smoke (file picker → generate → done state → Open in Finder) succeeds against `crates/panops-engine/tests/fixtures/audio/en_30s.wav`.
6. Quitting the app SIGTERMs the engine within 5 seconds.
7. CI's `swift build` step on `macos-latest` is green.
8. `cargo test --workspace --locked` still green (no Rust regression).
9. `clippy` and `rustfmt` still clean.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `NWConnection` over UDS + WebSocket upgrade is fiddly (Apple's docs are sparse) | Medium | Fall back to a hand-rolled WebSocket frame parser if `NWProtocolWebSocket` doesn't accept UDS. The frame surface needed is text-only, no extensions. Spike before locking the IPC client design. |
| SwiftPM SwiftUI app doesn't build cleanly without an Xcode project on macOS 14 | Low | SwiftUI apps under SwiftPM have been stable since Swift 5.5. If broken, fall back to Xcode project at slice 09b. |
| Engine cold-start race (app connects before socket exists) | Medium | Retry connection with exponential backoff up to 5s. Documented in `IpcClient.init`. |
| Stale socket from previous engine that didn't shut down cleanly | Low | Engine's `stale_socket_is_unlinked_and_rebound` test covers this engine-side. Swift app surfaces the error to the user if the engine fails to bind. |
| User picks an unsupported audio format and `job.error` doesn't surface a friendly message | Low | The engine's error kinds (`input_not_found`, `invalid_input`) are mapped to readable strings in the UI. File a follow-up if the message is unhelpful. |

## Open questions (deferred to future slices)

1. **Meetings list** — when slice 11 ships the master/detail layout. Already exists in the engine (`meeting.list`).
2. **Per-meeting language toggle UI** — north-star requirement; slice 11+ candidate.
3. **In-app markdown rendering** — `NSAttributedString` from markdown is built-in on macOS 12+, so when needed it's cheap.
4. **Live transcript view** — Anchor B will deliver `asr.partial` / `asr.final` events; UI consumes them.
5. **Engine binary discovery without env var in dev** — once the .app bundle exists with a copy of the binary, the env var becomes dev-only and the production path is `Bundle.main`. File a follow-up to make the dev path use a SwiftPM resource or symlink so the env var can be dropped from the default dev flow.
