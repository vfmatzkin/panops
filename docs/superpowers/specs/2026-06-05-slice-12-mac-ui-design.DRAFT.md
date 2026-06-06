# Slice 12 — Mac Shell UI Completion

**Status:** DRAFT (awaiting maintainer approval)
**Date:** 2026-06-05
**Author:** Claude (autonomous drafting agent; maintainer revision expected)
**Predecessor:** [slice 10 design](2026-05-12-slice-10-whisperkit-asr-sidecar-design.md)
**North-star tie-in:** Completes Anchor A's UI surface. v0.1 acceptance criteria #2 ("stop recording → diarized transcript appears in the app") and #4 ("notes file persists across app restarts") are unmet until this lands. The Mac shell becomes a usable recording app, not just a file-picker skeleton.

## Problem

After slice 10, the Mac shell (`apps/Panops/`) is a walking skeleton with three states: pick audio file → call `notes.generate` → show the output path. The UI does not:

1. **Show a transcript** — diarized segments from the pipeline are invisible to the user.
2. **Render notes in-app** — the user must open Finder to view the markdown.
3. **Show a meeting list** — v0.1 criterion #4 requires "notes file persists across app restarts", implying users browse past meetings.
4. **Support record/stop** — live capture is Anchor B, but the UI must be ready for the `recording.start` / `recording.stop` IPC methods slice 11 will introduce.
5. **Display screenshots** — screenshot-anchored notes are the product's core differentiator; the timeline/thumbnails view is missing.

Additionally, three `release:v0.1` debts must fold into this slice:

- **#122**: Replace HTTP-POST + filesystem-polling with WebSocket event-driven IPC client.
- **#123**: Drop the external `swift-testing` SwiftPM dependency (Swift 6.x has it built-in).
- **#124**: Non-fatal engine connection failure UX — the app should not quit on IPC connect failure.

## Goal

Expand the three-state skeleton into a usable Mac app UI with:

1. A **meeting list sidebar** showing past meetings (read via `meeting.list`), satisfying v0.1 criterion #4.
2. A **transcript view** showing diarized segments for the selected meeting.
3. An **in-app markdown renderer** for generated notes.
4. A **record/stop control bar** stubbed behind a `RecordingController` protocol so slice 12 ships independently of slice 11.
5. A **screenshot thumbnails strip** below the transcript.
6. Event-driven IPC via WebSocket (`events.subscribe`), closing #122.
7. Graceful engine-connect failure UX (#124) — show a retry prompt, don't quit.
8. Remove `swift-testing` external dep (#123), use built-in Swift Testing.

This slice is Swift-only. No Rust changes. It can land in parallel with slice 11 (which owns the engine-side recording IPC methods).

## Decisions

| # | Decision | Reason |
|---|---|---|
| D1 | **Navigation: master/detail with sidebar** — `MeetingListView` (sidebar) + `MeetingDetailView` (main content) | Standard macOS pattern; matches the v0.1 "browse past meetings" UX implied by criterion #4. Sidebar shows meeting summaries from `meeting.list`; detail view shows transcript + notes + screenshots for the selected meeting. |
| D2 | **Transcript view: plain `Text` rendering first** — segment list with speaker labels + timestamps | `AttributedString` from markdown is built-in on macOS 12+, but transcript rendering is simpler: just show `Speaker X: [text] (00:12–00:45)` lines. No rich-text needed. Deferred: folding/collapsing, search, copy-per-segment. |
| D3 | **Notes rendering: `Text` with Markdown via `AttributedString`** — load the markdown file and render with system markdown parser | macOS 12+ supports `AttributedString(markdown:)` natively. No external markdown renderer dep. The notes file is at `<meeting.dir_path>/notes.md` — read via `FileManager` (not IPC). |
| D4 | **Screenshot thumbnails: `LazyVGrid` of images** — read from `<meeting.dir_path>/screenshots/` directory | Screenshot paths aren't in IPC yet (deferred to Anchor B). For slice 12, we enumerate the directory and show thumbnails. Clicking opens in Preview.app via `NSWorkspace`. |
| D5 | **Record/stop boundary: `RecordingController` protocol** with a `MockRecordingController` impl for slice 12, `LiveRecordingController` in slice 11 | Slice 12 must ship before slice 11's `recording.*` IPC methods exist. The protocol lets the UI land now with mock behavior (shows a placeholder "recording not implemented" state). Slice 11 swaps in the real impl. |
| D6 | **WebSocket IPC client: hand-rolled RFC 6455 over UDS** — reuses the `NWConnection` from slice 09, adds WebSocket upgrade framing | Slice 09's amendment proved `NWProtocolWebSocket.Options` fails over UDS. The engine accepts manual HTTP upgrade to WebSocket. Slice 12 implements a minimal frame parser (text frames only, no extensions, no binary). |
| D7 | **IPC event routing: `EventStreamActor`** — central dispatcher mapping `job.done`/`job.error` to per-meeting callbacks | The WebSocket client receives events from `events.subscribe`. One actor holds the subscription and routes events to UI callbacks via `AsyncStream`. Decouples the client from view-model state updates. |
| D8 | **Engine connect failure: show "Retry" button, don't quit** — `AppViewModel.state` gains `.engineNotConnected` state with manual retry action | Slice 09's `bootstrap()` shows a fatal alert and quits. v0.1 UX (#124) requires non-fatal: the app stays open, the user retries. Engine spawn still succeeds; only IPC connect fails. |
| D9 | **Swift Testing: use built-in `Testing` module** — drop the external `swift-testing` package from `Package.swift` | Swift 6.0+ includes `Testing` as a standard module. The external package emits deprecation warnings. Change: remove the dependency line; import `Testing` directly (no package product needed). |
| D10 | **Container/presentational split: `MeetingDetailView` is a container; `TranscriptView`, `NotesView`, `ScreenshotsStrip` are presentational** | Container handles IPC calls and state; presentational views receive data via props. Improves testability (presentational views are pure SwiftUI, testable via `PreviewProvider` assertions). |
| D11 | **Transcript data source: read `<meeting.dir_path>/transcript.json`** — not via IPC | The `transcript.json` is written by the engine during `notes.generate` (see handlers.rs:515-529). No IPC method exposes transcript content today. The Mac shell reads the file directly. Deferred: IPC method for transcript fetch if live-capture needs real-time. |
| D12 | **No per-meeting language toggle UI this slice** — defer to slice 13+ | North-star requires per-meeting language toggle, but the UI surface is already large. Language selection happens during meeting creation via `meeting.start({language: "en"})` — the detail view shows the meeting's `language` field, but no edit UI. File as debt. |
| D13 | **Meeting list polling: on app launch + manual refresh button** — not WebSocket-driven | No `meeting.created` / `meeting.deleted` events exist. The sidebar fetches `meeting.list` on launch and on button click. Future slice could add events. |
| D14 | **Screenshot directory enumeration: best-effort, graceful empty** — if `screenshots/` subdir missing or empty, show "No screenshots" placeholder | The screenshots surface depends on Anchor B (live capture). Slice 12 shows a placeholder; no screenshots fixture exists for test meetings. |

## Scope

### In

1. **New Swift views** under `Sources/Panops/Views/`:
   - `MeetingListView.swift` — sidebar listing meetings from `meeting.list`.
   - `MeetingDetailView.swift` — container for the selected meeting's content.
   - `TranscriptView.swift` — segment list with speaker labels + timestamps.
   - `NotesView.swift` — markdown rendering via `AttributedString`.
   - `ScreenshotsStripView.swift` — thumbnail grid from `screenshots/` directory.
2. **New Swift models/actors**:
   - `RecordingController.swift` — protocol + mock impl.
   - `Transcript.swift` — Codable struct mirroring `panops-protocol`'s segment shape.
   - `EventStreamActor.swift` — WebSocket event dispatcher.
3. **IpcClient.swift refactor**:
   - Add `wsConnect()` method for WebSocket upgrade.
   - Add `subscribeEvents()` returning `AsyncStream<IpcEvent>`.
   - Add `meetingList()` method.
   - Deprecate filesystem-polling path in `AppViewModel`.
4. **Package.swift change**:
   - Remove `swift-testing` external dependency.
5. **AppViewModel.swift changes**:
   - New `.engineNotConnected` state with retry action.
   - New `.noMeetingSelected` state for master/detail navigation.
   - Integration with `EventStreamActor` for event-driven completion.
6. **ContentView.swift refactor**:
   - Replace single-window three-state UI with `NavigationSplitView` (sidebar + detail).
7. **Tests**:
   - WebSocket frame parser unit tests in `IpcClientCodecTests.swift`.
   - Event routing tests in new `EventStreamTests.swift`.
8. **README.md update**:
   - Document the WebSocket upgrade approach.
   - Document the mock recording controller for parallel landing.

### Out (filed as debt if surfaced)

- **Live capture (Anchor B)** — ScreenCaptureKit + audio + screenshot sampling.
- **Real recording IPC** (`recording.start` / `recording.stop`) — slice 11 owns.
- **Per-meeting language toggle UI** — north-star requirement; defer to slice 13+.
- **In-progress meeting UI** — meeting currently being recorded (shows "live" indicator).
- **Meeting creation from UI** — `meeting.start` is called today for notes generation; explicit "new meeting" button deferred.
- **Meeting deletion from UI** — `meeting.delete` IPC exists but no UI button this slice.
- **Transcript search/filter** — deferred polish.
- **Notes edit-in-app** — markdown is read-only this slice.
- **Screenshot drag-and-drop** — deferred polish.
- **Reconnect on WebSocket drop** — if WS drops mid-session, show error state; manual retry.
- **Multiple concurrent jobs** — one `notes.generate` at a time; concurrent job UI deferred.
- **Progress bar during notes generation** — no `job.progress` events yet.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  apps/Panops/Sources/Panops/                                    │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ ContentView (NavigationSplitView)                       │   │
│  │  ┌──────────────┐ ┌──────────────────────────────────┐ │   │
│  │  │ MeetingList  │ │ MeetingDetailView (container)    │ │   │
│  │  │ (sidebar)    │ │  ┌─────────────────────────────┐ │ │   │
│  │  │              │ │  │ RecordBar (stub/mock)       │ │ │   │
│  │  │ - meeting.list │ │  └─────────────────────────────┘ │ │   │
│  │  │ - select →  │ │  ┌─────────────────────────────┐ │ │   │
│  │  │   detail    │ │  │ TranscriptView              │ │ │   │
│  │  │             │ │  │  - segments from JSON       │ │ │   │
│  │  │             │ │  └─────────────────────────────┘ │ │   │
│  │  │             │ │  ┌─────────────────────────────┐ │ │   │
│  │  │             │ │  │ NotesView                   │ │ │   │
│  │  │             │ │  │  - AttributedString(md)     │ │ │   │
│  │  │             │ │  └─────────────────────────────┘ │ │   │
│  │  │             │ │  ┌─────────────────────────────┐ │ │   │
│  │  │             │ │  │ ScreenshotsStrip            │ │ │   │
│  │  │             │ │  │  - thumbnails from dir      │ │ │   │
│  │  │             │ │  └─────────────────────────────┘ │ │   │
│  │  └──────────────┘ └──────────────────────────────────┘ │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ AppViewModel (state machine)                            │   │
│  │  .engineNotConnected → retry                            │   │
│  │  .noMeetingSelected                                     │   │
│  │  .meetingSelected(id)                                   │   │
│  │  .notesGenerating → spinner                             │   │
│  │  .notesGenerated → show content                         │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ IpcClient (actor)                                       │   │
│  │  - HTTP POST for one-shot requests (meeting.list, etc)   │   │
│  │  - WebSocket for events.subscribe                        │   │
│  │  - wsConnect() → upgrade to WS                           │   │
│  │  - subscribeEvents() → AsyncStream<IpcEvent>             │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ EventStreamActor                                        │   │
│  │  - holds WS subscription                                 │   │
│  │  - routes job.done/job.error to callbacks               │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ RecordingController (protocol)                          │   │
│  │  - start() / stop() / isRecording                        │   │
│  │  - MockRecordingController (slice 12)                    │   │
│  │  - LiveRecordingController (slice 11)                    │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ WebSocket + HTTP POST over UDS
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  panops-engine serve (existing Rust, unchanged)                │
│  - ipc.events.subscribe → job.done / job.error                 │
│  - ipc.meeting.list / .get / .start                            │
│  - ipc.notes.generate                                          │
└─────────────────────────────────────────────────────────────────┘
```

## Components (Swift side)

### `MeetingListView`

Sidebar showing meetings from `meeting.list`. Each row shows title + timestamp. Selecting a row sets `AppViewModel.state = .meetingSelected(id)` and triggers `meetingGet(id)` + filesystem reads for transcript/notes/screenshots.

```swift
struct MeetingListView: View {
    @ObservedObject var vm: AppViewModel
    var meetings: [MeetingSummary]  // from meeting.list

    var body: some View {
        List(meetings, selection: $vm.selectedMeetingId) { meeting in
            VStack(alignment: .leading) {
                Text(meeting.title).font(.headline)
                Text(meeting.startedAt).font(.caption)
            }
        }
        .toolbar { Button("Refresh") { await vm.refreshMeetings() } }
    }
}
```

### `MeetingDetailView`

Container that orchestrates data loading for the selected meeting. Calls `meetingGet(id)` to get the `Meeting` struct, then reads `transcript.json`, `notes.md`, and enumerates `screenshots/` directory.

```swift
struct MeetingDetailView: View {
    let meeting: Meeting
    @State private var transcript: Transcript?
    @State private var notesContent: String?
    @State private var screenshots: [URL]?

    var body: some View {
        VStack {
            RecordBar(meeting: meeting, controller: vm.recordingController)
            TranscriptView(transcript: transcript)
            NotesView(content: notesContent)
            ScreenshotsStrip(urls: screenshots)
        }
        .task { await loadMeetingData() }
    }
}
```

### `TranscriptView`

Presentational. Receives a `Transcript` struct (decoded from JSON) and renders segment lines.

```swift
struct TranscriptView: View {
    let transcript: Transcript?

    var body: some View {
        if let t = transcript {
            ScrollView {
                LazyVStack(alignment: .leading) {
                    ForEach(t.segments, id: \.self) { seg in
                        HStack {
                            Text("[\(seg.startMs.s–seg.endMs.s)]")
                            Text("Speaker \(seg.speaker ?? "?"):")
                            Text(seg.text)
                        }
                    }
                }
            }
        } else {
            Text("No transcript").foregroundStyle(.secondary)
        }
    }
}
```

### `NotesView`

Presentational. Receives markdown string, renders with `AttributedString`.

```swift
struct NotesView: View {
    let content: String?

    var body: some View {
        if let md = content, !md.isEmpty {
            ScrollView {
                Text(AttributedString(markdown: md, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
                    .textSelection(.enabled)
            }
        } else {
            Text("No notes").foregroundStyle(.secondary)
        }
    }
}
```

### `ScreenshotsStripView`

Presentational. Receives URLs, shows thumbnail grid.

```swift
struct ScreenshotsStripView: View {
    let urls: [URL]?

    var body: some View {
        if let urls, !urls.isEmpty {
            ScrollView(.horizontal) {
                LazyHGrid(rows: [GridItem(.fixed(80))]) {
                    ForEach(urls, id: \.self) { url in
                        Image(url, width: 120, height: 80)
                            .onTapGesture { NSWorkspace.shared.open(url) }
                    }
                }
            }
        } else {
            Text("No screenshots").foregroundStyle(.secondary)
        }
    }
}
```

### `RecordingController` (protocol)

```swift
protocol RecordingController {
    var isRecording: Bool { get }
    func start(meetingId: String) async throws
    func stop() async throws -> URL?  // audio file path
}

class MockRecordingController: RecordingController {
    var isRecording = false
    func start(meetingId: String) async throws {
        isRecording = true
        // Placeholder: shows "recording not implemented" alert after 2s
    }
    func stop() async throws -> URL? {
        isRecording = false
        return nil
    }
}
```

Slice 11 provides `LiveRecordingController` that calls `ipc.recording.start` / `ipc.recording.stop`.

### `EventStreamActor`

```swift
actor EventStreamActor {
    private var subscription: AsyncStream<IpcEvent>?
    private var callbacks: [String: (IpcEvent) -> Void] = [:]  // job_id -> callback

    func subscribe(client: IpcClient) async throws {
        subscription = await client.subscribeEvents()
        for await event in subscription! {
            route(event)
        }
    }

    func registerCallback(jobId: String, handler: @escaping (IpcEvent) -> Void) {
        callbacks[jobId] = handler
    }

    func unregisterCallback(jobId: String) {
        callbacks.removeValue(forKey: jobId)
    }

    private func route(event: IpcEvent) {
        switch event {
        case .jobDone(let jobId, _), .jobError(let jobId, _):
            callbacks[jobId]?(event)
            callbacks.removeValue(forKey: jobId)
        case .unknown:
            // Ignore unknown events
            break
        }
    }
}
```

### `IpcClient` additions

New methods:

```swift
actor IpcClient {
    // Existing HTTP POST methods stay
    func meetingList() async throws -> [MeetingSummary]
    func meetingGet(id: String) async throws -> Meeting

    // WebSocket path
    private var wsConn: NWConnection?
    func wsConnect() async throws  // HTTP upgrade to WebSocket
    func subscribeEvents() async -> AsyncStream<IpcEvent>
}
```

WebSocket upgrade flow:

1. Open `NWConnection` to UDS.
2. Send HTTP GET with `Upgrade: websocket` + `Connection: Upgrade` + `Sec-WebSocket-Key`.
3. Read HTTP 101 response.
4. Switch to WebSocket frame parser for subsequent reads.
5. Send `{"jsonrpc":"2.0","id":N,"method":"ipc.events.subscribe","params":[]}` as a text frame.
6. Receive text frames, decode as `IpcEvent`.

Frame parser (minimal):

- Reads frame header (FIN, opcode, payload length).
- For text frames (opcode 0x01), unmask and decode JSON.
- Ignores binary frames, ping/pong (responds pong automatically), close frames.

## Data flow (event-driven notes generation)

1. User selects an existing meeting from sidebar OR creates new via "Generate notes" on an audio file.
2. `AppViewModel.notesGenerate(audio, meetingId)` → HTTP POST → `{job_id}`.
3. `EventStreamActor.registerCallback(jobId, handler)` → handler updates state when event arrives.
4. WebSocket receives `job.done` → handler called → `AppViewModel.state = .notesGenerated(meetingId)`.
5. `MeetingDetailView.task` loads transcript + notes + screenshots from filesystem.

## Error paths

| Scenario | Behavior |
|---|---|
| Engine binary not found at startup | Fatal alert (unchanged from slice 09). Quit on dismiss. |
| IPC connect fails after engine spawn | New `.engineNotConnected` state. Show "Could not connect to engine" + "Retry" button. Retry calls `client.connect()` again. |
| WebSocket drops during subscription | Show "Lost connection to engine" in the meeting detail. Manual "Retry" reconnects. |
| `transcript.json` missing for meeting | Show "No transcript" placeholder. |
| `notes.md` missing | Show "No notes" placeholder. |
| `screenshots/` empty | Show "No screenshots" placeholder. |
| `meeting.list` returns empty | Sidebar shows "No meetings yet". Placeholder text. |
| `notes.generate` RPC error | Show error in meeting detail view. Retry button. |

## Testing

### Unit (Swift)

1. **WebSocket frame parser tests** — valid text frame decodes, masked payload unmasks correctly, oversized length is rejected (per RFC 6455 limits).
2. **Event routing tests** — `EventStreamActor` routes `job.done` to registered callback, ignores `unknown`.
3. **Transcript JSON decode** — valid `transcript.json` fixture decodes to `Transcript` struct with correct segments.
4. **AttributedString markdown** — valid markdown renders without crash; malformed markdown shows raw text.

### Manual smoke (pre-PR)

1. Launch app with engine running.
2. Sidebar shows meetings (use `meeting.start` via CLI to seed test data).
3. Select a meeting — transcript + notes appear.
4. "Generate notes" on an audio file → spinner → notes appear in detail view.
5. Kill engine → "Retry" button appears → restart engine → retry succeeds.
6. No screenshots placeholder shows (expected for pre-Anchor-B meetings).

### CI

- `swift build` on macos-latest.
- `swift test` runs unit tests (WebSocket parser + event routing + decode tests).

## Three-tier boundaries

### ✅ Always do

- Run `swift build && swift test` clean before each commit.
- Keep `IpcClient` changes additive (HTTP POST methods remain; WS is addition).
- Use `Bundle.main.bundleURL` for production paths.
- File a debt issue for any "deferred" item (per-meeting language toggle, meeting creation UI, etc.).
- Commit per task in the slice plan.
- Remove the `swift-testing` external dependency.

### ⚠️ Ask first

- Adding a new SwiftPM dependency (e.g., a markdown renderer).
- Adding a new IPC method call beyond `meeting.list`, `meeting.get`, `notes.generate`, `events.subscribe`.
- Changing the WebSocket frame parser beyond minimal text-frame support.
- Adding a second window or sheet (modal).
- Adding `recording.start` / `recording.stop` real implementation (that's slice 11).

### 🚫 Never do

- Touch Rust code (this slice is Swift-only).
- Implement live capture (Anchor B).
- Add `recording.*` IPC calls (slice 11 owns).
- Phone home. Zero telemetry.
- Auto-merge the PR.
- Quit the app on IPC connect failure (non-fatal UX per #124).
- Remove the existing HTTP POST transport — it's still needed for one-shot requests.

## Acceptance criteria

1. `cd apps/Panops && swift build --configuration release` succeeds.
2. `cd apps/Panops && swift test` succeeds — WebSocket parser + event routing tests pass.
3. App launches, shows sidebar with meetings from `meeting.list`.
4. Selecting a meeting shows transcript (from `transcript.json`) + notes (from `notes.md`).
5. "Generate notes" on an audio file triggers notes generation, spinner shows, notes appear via event-driven completion (not filesystem polling).
6. If engine spawn succeeds but IPC connect fails, app shows "Retry" button instead of quitting.
7. No screenshots placeholder shows for meetings without `screenshots/` subdir.
8. `Package.swift` has no `swift-testing` external dependency.
9. CI's `swift build` + `swift test` pass on macos-latest.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| WebSocket upgrade over UDS is fiddly (slice 09's `NWProtocolWebSocket` failure) | Medium | Hand-roll a minimal frame parser. The engine's jsonrpsee accepts HTTP upgrade; spike before locking. |
| `AttributedString(markdown:)` handles `NotionEnhanced` dialect poorly (callout blocks, toggle lists) | Medium | Test with a fixture early. If rendering breaks, show raw markdown in a `Text` fallback. |
| Transcript JSON shape differs from Swift `Transcript` struct assumptions | Low | Read `handlers.rs` transcript serialization to match field names. |
| Event-driven UX feels slower than polling (latency perception) | Low | WebSocket is faster than 2s polling. Event arrives immediately on job completion. |
| Mock recording controller UX is confusing ("why can't I record?") | Medium | Show clear placeholder: "Recording requires live capture (Anchor B)". Button disabled. |

## Open questions

1. **Transcript JSON vs IPC method** — should slice 12 add `meeting.get_transcript` IPC method, or is filesystem read sufficient for v0.1? Filesystem read is simpler; IPC method would align with future live-capture needs.

2. **Screenshots metadata** — screenshots are just files in `screenshots/` today. Should slice 12 add a `screenshots.json` manifest (timestamps, captions) that Anchor B populates? Or defer to Anchor B entirely?

3. **Meeting creation UI** — slice 12 shows existing meetings. Should "New meeting" button (calls `meeting.start`) land here, or defer? Current design defers; `notes.generate` auto-creates meetings.

4. **Event stream lifecycle** — when should `EventStreamActor.subscribe()` be called? On app launch (always connected) or lazily when first job starts? Always-on risks socket leak on idle; lazy risks missed events if job starts before subscription.

5. **WebSocket reconnect** — if WS drops mid-session, should auto-reconnect attempt? Current design: manual retry. Auto-reconnect may race with in-flight job state.

6. **Multiple meeting selection** — should sidebar support multi-select (bulk delete)? Deferred polish, but worth flagging now.

---

## Appendix: Transcript JSON shape (from handlers.rs)

The engine writes `transcript.json` with this shape (see handlers.rs:515-529):

```json
{
  "schema_version": "1.0.0",
  "model": "whisper-base",
  "audio_path": "/path/to/audio.wav",
  "audio_duration_ms": 180000,
  "diarized": true,
  "segments": [
    {
      "start_ms": 0,
      "end_ms": 5000,
      "text": "Hello world",
      "speaker": "SPEAKER_01"
    },
    ...
  ]
}
```

The Swift `Transcript` struct must match this shape. `segments` is an array; each segment has `start_ms`, `end_ms`, `text`, and optionally `speaker`.