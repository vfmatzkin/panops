# Panops — macOS Shell

The macOS native app for Panops, a local-first recorder with screenshot-anchored meeting notes.

## Features

- **Meeting list sidebar** — browse past meetings from `meeting.list`
- **Transcript view** — diarized segments with timestamps and speaker labels
- **Notes rendering** — in-app markdown display with fallback for parse errors
- **Screenshot thumbnails** — click to open in Preview
- **Notes generation** — open audio file → generate notes via IPC

## Architecture

```
ContentView (NavigationSplitView)
├── MeetingListView (sidebar)
│   └── meeting.list → row selection
└── MeetingDetailView (detail)
    ├── RecordBar (stub/mock)
    ├── TranscriptView
    ├── NotesView
    └── ScreenshotsStripView
```

## IPC Transport

- **HTTP POST** for one-shot requests (`meeting.list`, `meeting.get`, `notes.generate`)
- **WebSocket** for event subscription (`events.subscribe`) — hand-rolled RFC 6455 upgrade over UDS

Slice 12 addressed debt #122 (WebSocket IPC), #124 (non-fatal engine connect failure UX).

Note: Tests use the `Testing` module bundled in Swift 6.x — no external swift-testing dependency (dropped in #123).

## Recording Controller

The RecordBar is disabled this slice with placeholder text. The RecordingController protocol abstracts recording so slice 12 can ship independently of slice 11's recording.* IPC methods.

- MockRecordingController — placeholder impl (this slice)
- LiveRecordingController — real impl (slice 11/future)

## Development

```bash
cd apps/Panops
swift build
swift run
```

The app connects to panops-engine serve over Unix socket at ~/Library/Application Support/panops/engine.sock.

Set PANOPS_ENGINE_BIN to override the engine binary path for development.

## Tests

```bash
swift test
```

WebSocket frame parser tests and event routing tests validate the IPC implementation.

## Installation (Homebrew)

Install via the custom tap:

```bash
brew install --cask vfmatzkin/panops/panops
```

The cask installs `Panops.app` to `/Applications`. Panops is ad-hoc signed (not
Apple-notarized), so macOS Gatekeeper will block first launch. Clear the quarantine
flag once:

```bash
xattr -dr com.apple.quarantine /Applications/Panops.app
```

Or right-click the app → Open, then confirm in System Settings → Privacy & Security.
