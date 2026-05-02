# panops IPC protocol — slice 05

## Transport

- Socket path: `~/Library/Application Support/panops/engine.sock` (override with `panops-engine serve --socket <path>`).
- Permissions: `0600` (user-private). No token auth.
- Framing: JSON-RPC 2.0 control plane + WebSocket event plane multiplexed on the same connection. WS upgrade detected by `Upgrade: websocket` header.
- Stale-socket recovery: on `serve` start, the engine connects to the path. If the connection succeeds it exits with "engine already running"; otherwise it unlinks the file and binds.

## Methods

| Method | Params | Result | Notes |
|---|---|---|---|
| `ipc.notes.generate` | `{ audio, dialect?, llm_provider?, llm_model?, no_diarize?, language? }` | `{ job_id }` | Async. Listen on `ipc.events.subscribe` for `job.done` / `job.error`. |
| `ipc.meeting.list` | `()` | `[]` (until #17) | Returns array of `MeetingSummary`. |

The `ipc.` namespace + `.` separator are wired via jsonrpsee `#[rpc(server, namespace = "ipc", namespace_separator = ".")]`.

`NotesGenerateParams` fields (all but `audio` are optional):

- `audio` — absolute or working-dir-relative path to the audio file.
- `dialect` — `"notion-enhanced"` (default) or `"basic"`.
- `llm_provider` — provider name string (e.g. `"ollama"`). Slice-04 wiring picks the default if absent.
- `llm_model` — provider-specific model id (e.g. `"gemma3:4b"`).
- `no_diarize` — skip the diarization merge step.
- `language` — BCP-47 language hint passed to ASR.

Param structs intentionally do NOT use `#[serde(deny_unknown_fields)]` — same forward-compat philosophy as `IpcError::Unknown`. New optional fields are non-breaking.

## Subscriptions

| Subscription | Item type | Lifetime |
|---|---|---|
| `ipc.events.subscribe` | `Event` | Until client unsubscribes (`ipc.events.unsubscribe`) or connection closes. Late subscribers miss earlier events; replay deferred. |

The subscription is server-push backed by a `tokio::sync::broadcast` channel. A lagging subscriber drops events but keeps the subscription open (one missed event beats tearing down the WS).

## Event types

```json
{ "type": "job.done",  "job_id": "...", "result": { "primary_file": "...", "assets": [...] } }
{ "type": "job.error", "job_id": "...", "error": { "kind": "input_not_found" | "invalid_input" | "provider_unavailable" | "internal" | "cancelled", "message": "..." } }
```

The `Event` enum is internally tagged on `type`. Future event kinds (`asr.partial`, `asr.final`, `screenshot`, `job.progress`) extend this enum. Old clients deserialise unrecognised tags as `Event::Unknown(<original JSON>)`, preserving the subscription so one new tag does not tear down older clients. Implementations that do not use the Rust types directly should mirror this: any envelope whose `type` is not in the known set should be logged and skipped, never treated as a fatal protocol error.

## Error taxonomy

`IpcError` ships with five `kind`s plus a forward-compat `unknown` fallback:

| `kind` | Meaning | Payload |
|---|---|---|
| `input_not_found` | A path the engine was told to read does not exist. | `path` |
| `invalid_input` | A request param failed validation. | `message` |
| `provider_unavailable` | An external LLM/STT provider was unreachable or returned empty. | `message` |
| `internal` | Engine-side bug or unrecognised failure. | `message` |
| `cancelled` | Operation was cancelled (post-slice-05). | (none) |
| `unknown` | Forward-compat fallback when an old client sees a new variant the engine added later. | (none) |

Adding new variants is non-breaking for existing clients (they deserialise as `unknown`). Renaming or removing variants IS breaking.

At the JSON-RPC boundary, errors flowing back as `ErrorObjectOwned` use code `-32000` with `IpcError` shape preserved in the `data` field; `notes.generate` reports per-job failures via `job.error` events on the subscription instead.

## What's NOT shipped in slice 05

- Control methods: `meeting.start` / `meeting.stop` / `meeting.get` / `meeting.delete` / `meeting.set_language`, `asr.post_pass` / `asr.cancel`, `notes.export`, `llm.probe` / `llm.providers` / `llm.test`, `settings.get` / `settings.set`.
- Live-capture events: `asr.partial`, `asr.final`, `screenshot`, `job.progress`.
- Token auth, WS reconnection, event replay buffer.
- `CancellationToken` plumbed through `LlmRequest` (the `spawn_blocking` task is uncancellable today).

Each deferred item has a tracking issue under `type:debt area:ipc` on the project board.

## Manual smoke

```bash
# Terminal 1
panops-engine serve --socket /tmp/panops.sock

# Terminal 2
websocat unix-connect:/tmp/panops.sock
> {"jsonrpc":"2.0","id":1,"method":"ipc.meeting.list","params":{}}
< {"jsonrpc":"2.0","id":1,"result":[]}
```

Until #74 is resolved, `serve` uses stub adapters — `notes.generate` will return errors via `job.error` instead of producing real notes. In-process integration tests under `crates/panops-engine/tests/` exercise the real path with injected adapters via `EngineServices`.
