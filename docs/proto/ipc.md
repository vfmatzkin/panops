# panops IPC protocol — slices 05 + 06

## Transport

- Socket path: `~/Library/Application Support/panops/engine.sock` (override with `panops-engine serve --socket <path>`).
- Permissions: `0600` (user-private). No token auth.
- Framing: JSON-RPC 2.0 control plane + WebSocket event plane multiplexed on the same connection. WS upgrade detected by `Upgrade: websocket` header.
- Stale-socket recovery: on `serve` start, the engine connects to the path. If the connection succeeds it exits with "engine already running"; otherwise it unlinks the file and binds.

## Methods

| Method | Params | Result | Notes |
|---|---|---|---|
| `ipc.notes.generate` | `{ audio, dialect?, llm_provider?, llm_model?, no_diarize?, language?, meeting_id? }` | `{ job_id }` | Async. Listen on `ipc.events.subscribe` for `job.done` / `job.error`. |
| `ipc.meeting.list` | `()` | `[MeetingSummary]` | Returns rows from the registry, ordered `started_at DESC`. |
| `ipc.meeting.start` | `MeetingConfig` | `meeting_id` (string) | Slice 06. Data-plane scaffolding (no live capture). Creates row + `meetings/<uuid>/screenshots/`. |
| `ipc.meeting.stop` | `{ id }` | `Meeting` | Sets `ended_at = now()` and computes `duration_ms`. |
| `ipc.meeting.get` | `{ id }` | `Meeting` | Full row including `dir_path`, `language`, `ended_at`. |
| `ipc.meeting.set_language` | `{ id, language }` | `Meeting` | Updates BCP-47 language hint. |
| `ipc.meeting.delete` | `{ id }` | `()` | Removes registry row (FK-cascades notes), then `rm -rf meetings/<id>/`. Orphan dir on FS-error is logged, not an error. |

The `ipc.` namespace + `.` separator are wired via jsonrpsee `#[rpc(server, namespace = "ipc", namespace_separator = ".")]`.

### `NotesGenerateParams`

| Field | Type | Default | Notes |
|---|---|---|---|
| `audio` | string | required | Absolute or working-dir-relative path to the audio file. Canonicalized server-side. |
| `dialect` | `"notion-enhanced"` \| `"basic"` | `"notion-enhanced"` | Output markdown dialect. |
| `llm_provider` | string | absent | E.g. `"ollama"`. Slice-04 wiring picks the default if absent. |
| `llm_model` | string | provider default | E.g. `"gemma3:4b"`. |
| `no_diarize` | bool | `false` | Skip the diarization merge step. |
| `language` | string | absent | BCP-47 language hint passed to ASR + LLM. |
| `meeting_id` | string | absent | **Slice 06.** When `Some`, attach the generated note to the existing meeting (its `dir_path` becomes the output dir). When `None`, the engine auto-creates a meeting under `<data_dir>/meetings/<uuid>/` and returns the new id in `result.meeting_id`. |

### `NotesGenerateResult` (delivered via `job.done` event)

| Field | Type | Notes |
|---|---|---|
| `primary_file` | string | Absolute path to the rendered `notes.md`. |
| `assets` | `[string]` | Additional written assets (screenshots, etc.). |
| `meeting_id` | string | **Slice 06.** Always set; either the value passed in `params.meeting_id` or the id of the auto-created meeting. |

### `Meeting` and `MeetingSummary`

```json
// MeetingSummary (slice 05; shape unchanged)
{ "id": "...", "title": "...", "started_at": "RFC3339", "duration_ms": 0 }

// Meeting (slice 06; new)
{
  "id": "...",
  "title": "...",
  "started_at": "RFC3339",
  "ended_at": null | "RFC3339",
  "duration_ms": null | u64,
  "language": "en" | "es" | "auto" | ...,
  "dir_path": "/Users/.../meetings/<uuid>"
}
```

In-progress meetings render `ended_at: null` and `duration_ms: null` on `Meeting`; `MeetingSummary.duration_ms` defaults to `0` to keep the slice-05 wire shape stable.

### `MeetingConfig` (input to `meeting.start`)

```json
{ "title": "Daily standup",  // optional, defaults to ""
  "language": "en"            // optional, defaults to "auto"
}
```

Both fields are optional; an empty `{}` is accepted and applies defaults.

Param structs intentionally do NOT use `#[serde(deny_unknown_fields)]` — same forward-compat philosophy as `IpcError::Unknown`. New optional fields are non-breaking.

## Subscriptions

| Subscription | Item type | Lifetime |
|---|---|---|
| `ipc.events.subscribe` | `Event` | Until client unsubscribes (`ipc.events.unsubscribe`) or connection closes. Late subscribers miss earlier events; replay deferred. |

The subscription is server-push backed by a `tokio::sync::broadcast` channel. A lagging subscriber drops events but keeps the subscription open (one missed event beats tearing down the WS).

## Event types

```json
{ "type": "job.done",  "job_id": "...", "result": { "primary_file": "...", "assets": [...], "meeting_id": "..." } }
{ "type": "job.error", "job_id": "...", "error": { "kind": "input_not_found" | "invalid_input" | "provider_unavailable" | "internal" | "cancelled", "message": "..." } }
```

The `Event` enum is internally tagged on `type`. Future event kinds (`asr.partial`, `asr.final`, `screenshot`, `job.progress`) extend this enum. Old clients deserialise unrecognised tags as `Event::Unknown(<original JSON>)`, preserving the subscription so one new tag does not tear down older clients. Implementations that do not use the Rust types directly should mirror this: any envelope whose `type` is not in the known set should be logged and skipped, never treated as a fatal protocol error.

## Error taxonomy

`IpcError` ships with five `kind`s plus a forward-compat `unknown` fallback:

| `kind` | Meaning | Payload |
|---|---|---|
| `input_not_found` | A path or id the engine was told to read does not exist. Slice 06 also uses this for unknown `meeting_id`s, with `path: "<kind>/<id>"` (e.g. `"meeting/abc"`). | `path` |
| `invalid_input` | A request param failed validation. | `message` |
| `provider_unavailable` | An external LLM/STT provider was unreachable or returned empty. | `message` |
| `internal` | Engine-side bug or unrecognised failure. | `message` |
| `cancelled` | Operation was cancelled (post-slice-05). | (none) |
| `unknown` | Forward-compat fallback when an old client sees a new variant the engine added later. | (none) |

Adding new variants is non-breaking for existing clients (they deserialise as `unknown`). Renaming or removing variants IS breaking.

At the JSON-RPC boundary, errors flowing back as `ErrorObjectOwned` use code `-32000` with `IpcError` shape preserved in the `data` field; `notes.generate` reports per-job failures via `job.error` events on the subscription instead.

## Storage (slice 06)

- Single SQLite file at `<data_dir>/panops.db` with `meeting` + `note` tables. Default `<data_dir>` is `~/Library/Application Support/panops/`; override with `panops-engine --data-dir <path>`.
- Per-meeting directory: `<data_dir>/meetings/<uuid>/notes.md` and `<data_dir>/meetings/<uuid>/screenshots/` (empty until live capture).
- Schema version: `PRAGMA user_version = 1`. A future-versioned DB is rejected on engine start with a clean exit code; the `domain-conversions` mapping translates the underlying `StorageError::SchemaMismatch` to `IpcError::Internal { message: "storage schema mismatch" }` for any IPC consumer that hits it later.
- Concurrency: single `Arc<Mutex<Connection>>`; all storage calls are wrapped in `tokio::task::spawn_blocking`. Single-user-fine for v0.1.

### Known limitations

- Per-meeting `meeting.db` (with `segment / speaker / screenshot / job` tables) is deferred to the live-capture slice.
- WAL mode is off; concurrent readers serialise behind the single mutex.
- Manually deleting `panops.db` orphans the per-meeting directories on disk. There is no orphan reaper; clean up by hand.
- No schema migration framework yet — version bumps require a v0.2 plan.
- `notes.generate` returning `meeting_id` is a wire **extension**, not a removal — clients that ignored the field continue to work.

## What's NOT shipped (still deferred)

- IPC: `asr.post_pass` / `asr.cancel`, `notes.export`, `llm.probe` / `llm.providers` / `llm.test`, `settings.get` / `settings.set`.
- Live-capture events: `asr.partial`, `asr.final`, `screenshot`, `job.progress`.
- Token auth, WS reconnection, event replay buffer.
- `CancellationToken` plumbed through `LlmRequest` (the `spawn_blocking` task is uncancellable today).
- Push events for storage mutations (`meeting.created`, `meeting.deleted`).

Each deferred item has a tracking issue under `type:debt area:ipc` (or `area:storage`) on the project board.

## Manual smoke

```bash
# Terminal 1
panops-engine serve --data-dir /tmp/panops-smoke --socket /tmp/panops.sock
```

The engine accepts JSON-RPC 2.0 over both raw HTTP and WebSocket on the
same UDS. **Use HTTP (curl) for one-off requests; use WebSocket
(websocat) when you need `events.subscribe` server-push.**

**curl (HTTP, simplest):**

```bash
# Terminal 2
curl -s --unix-socket /tmp/panops.sock \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"ipc.meeting.start","params":[{"title":"Test","language":"en"}]}' \
  http://localhost/
# < {"jsonrpc":"2.0","id":1,"result":"<uuid>"}

curl -s --unix-socket /tmp/panops.sock \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"ipc.meeting.list","params":[]}' \
  http://localhost/
# < {"jsonrpc":"2.0","id":2,"result":[{"id":"<uuid>","title":"Test","started_at":"...","duration_ms":0}]}
```

**websocat (WebSocket, for events):**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"ipc.meeting.list","params":[]}' \
  | websocat -n1 -t --ws-c-uri=ws://localhost/ - ws-c:unix:/tmp/panops.sock
```

The `- ws-c:unix:` proxy form is required by websocat 1.x (`-` is stdio
as the "other" endpoint of the proxy; `ws-c:` wraps the UDS transport
in a WS handshake).

**Params shape:** all `#[rpc]` methods take their params as a single
positional element in the JSON-RPC `params` array — `params: [<obj>]`,
NOT `params: <obj>`. jsonrpsee's macro generates positional dispatch by
default; named-object params would require an extra wrapper struct per
method.

Restart the engine pointing at the same `--data-dir`; `ipc.meeting.list` returns the same row (persistence verified).

In-process integration tests under `crates/panops-engine/tests/` exercise the full IPC surface with injected fakes via `EngineServices::ready` — see `tests/common/mod.rs` and `tests/common/notes_pipeline.rs` for the helper layout.
