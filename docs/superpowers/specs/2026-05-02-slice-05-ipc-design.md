# Slice 05 — IPC layer: design

**Status**: Locked 2026-05-02.
**Goal**: Land the JSON-RPC 2.0 control plane + WebSocket event plane over a Unix domain socket at `~/Library/Application Support/panops/engine.sock`, exercised by a Rust integration-test client. Walking-skeleton scope: two methods (`notes.generate`, `meeting.list`) and two events (`job.done`, `job.error`) — every other method/event from `2026-04-30-panops-design.md` §178–217 ships in a follow-up slice.

This slice is the LAST piece before the SwiftUI Mac shell (slice 06). It does not touch live capture, persistence (#17), or new ports beyond `panops-protocol`. It does land four cross-cutting decisions (binary shape, runtime topology, error transport, dependency stack) that the Mac app and live-capture slices both build on.

## Why this shape

A walking-skeleton slice is the smallest end-to-end path that exercises every load-bearing decision once. The four loads:

1. **Binary shape**: server-with-CLI-subcommands, not server-spawns-CLI. Default `panops-engine <wav>` keeps its stdout JSON contract for shell pipelines and CI smoke tests; new `serve` subcommand never writes to stdout. One binary, one dylib load, one tracing init.
2. **Runtime topology**: the engine binary owns *two* tokio runtimes — one for jsonrpsee (RPC accept + dispatch), one for outbound LLM HTTP. This separation isolates head-of-line blocking on slow LLM calls from RPC accept latency. Pipeline work is offloaded via `tokio::task::spawn_blocking` so rayon's intra-job parallelism never lands on a tokio worker thread.
3. **Error transport**: `panops-protocol` (new transport-only crate) hosts `IpcError` with `From<DomainErr>` impls behind a `domain-conversions` feature that pulls in `panops-core`. Domain crates stay serde-free; coherence holds; engine just calls `.into()`.
4. **Dependency stack**: `jsonrpsee 0.26` (server + macros features) plus the low-level `serve_with_graceful_shutdown` API to bind to `tokio::net::UnixListener`. WebSocket multiplexed on the same connection via `is_upgrade_request` branch.

If any of these is wrong, slice 06 will pay for it loudly. They're picked here so the next 12 control methods, 4 event types, and the `LlmProviderProbe` port land into a stable harness.

## Scope (in this slice)

- New crate `panops-protocol` with request/response types + `IpcError` + serde + thiserror. No transport code; pure types.
- `panops-engine serve [--socket <path>]` subcommand alongside existing default mode and `notes`.
- Two control methods:
  - `notes.generate({ audio: PathBuf, dialect?: "notion-enhanced" | "basic", llm_provider?: "auto" | "ollama", llm_model?: String, no_diarize?: bool, language?: String }) -> { job_id }` — runs the existing pipeline; emits `job.done` or `job.error` over WS when finished. Synchronous-completion (no incremental progress) for slice 05; `job.progress` defers to slice 07 with live capture.
  - `meeting.list() -> []` — returns empty array. Backed by SQLite once #17 lands; ships now to lock the response shape.
- Two event types over WebSocket:
  - `{ "type": "job.done", "job_id": String, "result": { "primary_file": PathBuf, "assets": [PathBuf] } }`
  - `{ "type": "job.error", "job_id": String, "error": IpcError }`
- Stale-socket cleanup with safety check (refuse if a live engine is listening; unlink if dead).
- Filesystem perms `0600` on the socket immediately after bind.
- Graceful shutdown: SIGINT/SIGTERM → close listener → drain in-flight RPCs → `serve` returns Ok.
- Rust integration test client driving real round-trips against an in-process server.

## Out of scope (defer; file as `type:debt area:ipc` issues at slice end)

- All other 12 control methods (`meeting.start/stop/get/delete/set_language`, `asr.post_pass/cancel`, `notes.export`, `llm.probe/providers/test`, `settings.get/set`).
- Live-capture events (`asr.partial`, `asr.final`, `screenshot`, `job.progress`).
- `Storage` port + SQLite (#17). `meeting.list` returns `[]` until that lands.
- `LlmProviderProbe` port (design §202–203).
- Auth tokens beyond `0600` filesystem perms.
- WebSocket reconnection / replay buffer / event backpressure tuning beyond defaults.
- Cancellation tokens through `LlmRequest` (file as `severity:low` debt; revisit when token-billing matters).
- Cross-platform UDS (Linux nice-to-have, Windows out of scope; design is mac-first).

## Architecture

### Binary topology

`panops-engine` keeps clap-based subcommand dispatch (slice 04 shape). Three modes:

| Mode | stdout contract | stderr | Purpose |
|---|---|---|---|
| `panops-engine <wav>` (default) | JSON transcript (existing, unchanged) | tracing | Shell pipelines, CI smoke. |
| `panops-engine notes <wav> [...]` | nothing | tracing | CLI notes generation, slice 04. |
| `panops-engine serve [--socket <path>]` (new) | nothing | tracing | IPC server for the Mac app and the test client. |

`run_serve(socket: Option<PathBuf>) -> Result<(), (u8, String)>` is the new entry point. It owns:

```
EngineServices {
    llm_factory: Arc<dyn Fn(...) -> Result<Arc<dyn LlmProvider>, IpcError> + Send + Sync>,
    asr_factory: Arc<dyn Fn(...) -> Result<Box<dyn AsrProvider>, IpcError> + Send + Sync>,
    diar_factory: Arc<dyn Fn(...) -> Result<Box<dyn Diarizer>, IpcError> + Send + Sync>,
    exporter: Arc<dyn NotesExporter + Send + Sync>,
    llm_handle: tokio::runtime::Handle,   // points at the LLM HTTP runtime
}
```

The factory pattern lets tests substitute real adapters with fakes (e.g., `MockLlm`, `TranscriptFileFake`) without env vars. CLI `serve` populates factories with `WhisperRsAsr`, `SherpaDiarizer`, `GenaiLlm::with_handle`, `MarkdownExporter`.

Default mode and `notes` continue to construct adapters directly (no factories — they're one-shot processes; the indirection is only paying its rent in `serve`).

### Runtime topology

Two tokio runtimes inside the `serve` binary:

```
+---------------------------------------+
| Runtime A: "rpc"                      |
|   - jsonrpsee server                  |
|   - UnixListener accept loop          |
|   - per-connection serve              |
|   - Subscription sinks (WS push)      |
+---------------------------------------+
            |
            | spawn_blocking(move || pipeline.generate(input))
            v
+---------------------------------------+
| Worker thread (tokio blocking pool)   |
|   - rayon::par_iter over sections     |
|   - each rayon worker:                |
|     llm.complete(req) -> block_on    -+--+
+---------------------------------------+   |
                                            v
                                +-------------------------+
                                | Runtime B: "llm-http"   |
                                |   - genai HTTP client   |
                                |   - reqwest / hyper     |
                                +-------------------------+
```

Why two runtimes:

- **Isolation of head-of-line blocking.** A 30-second LLM call must not delay an unrelated `meeting.list` RPC. Separating the runtimes guarantees the RPC accept thread never polls an LLM future.
- **Safe `block_on` from rayon workers.** `GenaiLlm::complete` calls `handle_b.block_on(async_http_call)`. Rayon workers are external OS threads relative to runtime B, so this is the standard "drive async from a sync thread" pattern (no nested-runtime panic, no deadlock).
- **Pipeline cancellation semantics.** Dropping the spawn_blocking JoinHandle does not cancel rayon work; the pipeline runs to completion and its result is dropped. Acceptable at single-user / ~5 concurrent RPC scale. Token-aware cancellation deferred (see Out of scope).

`GenaiLlm` API change:
- Old: `GenaiLlm::new(model)` → owns a private `Arc<Runtime>`.
- New: `GenaiLlm::new(model)` keeps a *lazy* private runtime (CLI users — default mode, `notes` subcommand, regen test). Internally `OnceLock<Runtime>`-style, one shared runtime across all CLI-side `GenaiLlm` instances in a process.
- New: `GenaiLlm::with_handle(model, handle: tokio::runtime::Handle)` for `serve` mode. The engine binary creates Runtime B once and threads its handle into the LLM factory.

### `panops-protocol` crate

```
crates/panops-protocol/
├── Cargo.toml            # serde, serde_json, thiserror; OPTIONAL feature: domain-conversions
├── src/
│   ├── lib.rs
│   ├── error.rs          # IpcError + From<DomainErr> behind feature flag
│   └── methods.rs        # NotesGenerateParams, NotesGenerateResult, MeetingSummary, JobId, JobDoneEvent, JobErrorEvent
```

`Cargo.toml` features:
```toml
[features]
default = []
domain-conversions = ["dep:panops-core"]

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
panops-core = { path = "../panops-core", optional = true }
```

`From<NotesError|AsrError|DiarError|LlmError> for IpcError` lives **in `panops-protocol::error`** (gated `#[cfg(feature = "domain-conversions")]`). Coherence is satisfied because `IpcError` is local to `panops-protocol`. `panops-engine` enables the feature; future Swift-side codegen targets that consume only the wire types do not.

`IpcError` shape:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IpcError {
    #[error("input not found: {path}")]
    InputNotFound { path: String },
    #[error("invalid input: {message}")]
    InvalidInput { message: String },
    #[error("provider unavailable: {message}")]
    ProviderUnavailable { message: String },
    #[error("internal: {message}")]
    Internal { message: String },
    #[error("cancelled")]
    Cancelled,
    #[serde(other)]
    #[error("unknown error kind (forward-compat fallback)")]
    Unknown,
}
```

The `Unknown` unit variant + `#[serde(other)]` keeps the wire format forward-compatible: a future engine adding `RateLimited` deserializes as `Unknown` on an old client, instead of failing the whole RPC response. Adding new variants is therefore non-breaking; removing or renaming existing ones is breaking.

Variant mapping (engine-side):

| Domain | → IpcError variant |
|---|---|
| `AsrError::AudioNotFound(p)` | `InputNotFound { path: p.display().to_string() }` |
| `AsrError::InvalidAudio(m)` | `InvalidInput { message: m }` |
| `AsrError::Model(m)` / `Transcription(m)` | `Internal { message: m }` |
| `AsrError::Io(e)` | `Internal { message: e.to_string() }` |
| `DiarError::AudioNotFound(p)` | `InputNotFound { path: p.display().to_string() }` |
| `DiarError::InvalidAudio(m)` | `InvalidInput { message: m }` |
| `DiarError::Model(m)` / `Diarization(m)` | `Internal { message: m }` |
| `DiarError::Io(e)` | `Internal { message: e.to_string() }` |
| `LlmError::Network(m)` / `Provider(m)` | `ProviderUnavailable { message: m }` |
| `LlmError::InvalidSchema { expected, got }` | `Internal { message: format!("schema mismatch: expected {expected}, got {got}") }` |
| `LlmError::EmptyResponse` | `ProviderUnavailable { message: "empty LLM response".into() }` |
| `LlmError::Cancelled` | `Cancelled` |
| `NotesError::EmptyTranscript` | `InvalidInput { message: "empty transcript".into() }` |
| `NotesError::Llm(e)` | recurse into `LlmError` mapping above |
| `NotesError::SchemaMismatch { stage, detail }` | `Internal { message: format!("schema mismatch in stage {stage}: {detail}") }` |
| `NotesError::InvalidInput(m)` | `InvalidInput { message: m }` |

### Transport: jsonrpsee + UDS

Pin `jsonrpsee = { version = "0.26", features = ["server", "macros"] }`. Transport entry point: `jsonrpsee::server::serve_with_graceful_shutdown(io, service, shutdown)` where `io: AsyncRead + AsyncWrite + Send + Unpin + 'static`. `tokio::net::UnixStream` satisfies those bounds.

Per-connection flow (one closure per accepted UDS connection):

1. Build a tower service via `Server::builder().to_service_builder()` + `RpcServiceBuilder`.
2. Wrap it: branch on `jsonrpsee::server::ws::is_upgrade_request(&req)`.
   - Upgrade: `ws::connect(req, server_cfg, methods, conn_state, rpc_middleware, conn_guard).await` — events flow over this connection.
   - Else: `http::call_with_service_builder(req, ...)` — single-shot JSON-RPC request/response.
3. Hand the future to `serve_with_graceful_shutdown(unix_stream, service, shutdown_notify)`.

Reference templates: `examples/jsonrpsee_server_low_level_api.rs` (UDS-shaped accept loop, copy verbatim with `TcpListener` → `UnixListener`) and `examples/ws_pubsub_broadcast.rs` (subscription-driven event push via `tokio::sync::broadcast`).

WebSocket events (`job.done`, `job.error`) ride a single `events` subscription:

```rust
#[rpc(server, namespace = "ipc")]
pub trait Ipc {
    #[method(name = "notes.generate")]
    async fn notes_generate(&self, params: NotesGenerateParams) -> Result<JobAccepted, IpcError>;

    #[method(name = "meeting.list")]
    async fn meeting_list(&self) -> Result<Vec<MeetingSummary>, IpcError>;

    #[subscription(name = "events.subscribe" => "events", item = Event)]
    async fn subscribe_events(&self) -> SubscriptionResult;
}
```

The event channel is a `tokio::sync::broadcast::Sender<Event>` shared across all connections. `notes.generate`'s spawn_blocking task posts `Event::JobDone { ... }` or `Event::JobError { ... }` on completion. Late subscribers miss earlier events (broadcast semantics) — acceptable for slice 05 because the test client subscribes before issuing the RPC. Replay deferred.

### Socket lifecycle

On `serve` start:
1. Resolve socket path: `--socket <path>` flag wins; default `~/Library/Application Support/panops/engine.sock`. Create parent directory with `0700` perms if missing.
2. Probe for live engine: `tokio::net::UnixStream::connect(&path).await`. If success → exit non-zero with `engine already running at <path>` (don't steal the socket).
3. If connect failed (file absent OR file present but nothing listening): `std::fs::remove_file(&path).ok()` (ignore error if absent).
4. Bind: `tokio::net::UnixListener::bind(&path)?`.
5. Set perms: `std::fs::set_permissions(&path, Permissions::from_mode(0o600))?`.
6. Install SIGINT/SIGTERM handlers via `tokio::signal::unix` that flip a `Notify` shared with `serve_with_graceful_shutdown`.

On `serve` shutdown:
1. `serve_with_graceful_shutdown` returns when the notify fires.
2. Drain accepted connections (jsonrpsee handles per-connection shutdown).
3. Best-effort `std::fs::remove_file(&path)` — don't fail shutdown on cleanup error, just log.

### Error mapping at the RPC boundary

Method handlers return `Result<T, IpcError>`. jsonrpsee converts `IpcError` to `ErrorObjectOwned` via `impl From<IpcError> for ErrorObjectOwned`:

```rust
impl From<IpcError> for ErrorObjectOwned {
    fn from(e: IpcError) -> Self {
        ErrorObjectOwned::owned(
            -32000,                       // application error code (server defined)
            e.to_string(),                // human-readable message from thiserror
            Some(serde_json::to_value(&e).expect("serialize IpcError")),
        )
    }
}
```

The structured `kind` survives in `error.data`, the `message` survives at top-level. Clients that parse `data` get the typed enum; clients that don't get a useful string.

## Test surface

Required tests (gate the slice-05 PR):

1. **`tests/ipc_server_starts_and_binds.rs`** — spawn `panops-engine serve --socket <tmp>` as a child, assert socket file appears with `0600` perms, connect and ping, send shutdown signal, assert binary exits 0 within 5s and socket is unlinked.
2. **`tests/ipc_notes_generate_round_trip.rs`** — in-process server with `MockLlm` injected via `EngineServices.llm_factory`. Subscribe to events, call `notes.generate` against the multi_speaker_60s fixture (ASR via `TranscriptFileFake`, diar via `KnownTurnsFake`), await `job.done`, assert primary file exists and matches expected schema.
3. **`tests/ipc_job_error_carries_kind.rs`** — call `notes.generate` with an audio path that doesn't exist; assert `job.error` arrives with `error.kind == "input_not_found"` and a usable message.
4. **`tests/ipc_stale_socket_is_cleaned.rs`** — pre-create a stale socket file (touch, no listener), start `serve`, assert it removes the stale file and binds successfully.
5. **`tests/ipc_refuses_to_steal_live_socket.rs`** — start two `serve` instances on the same path, assert second exits non-zero with a clear "engine already running" message.
6. **`tests/ipc_meeting_list_returns_empty.rs`** — call `meeting.list`, assert response is `[]` with the expected JSON shape.
7. **`tests/ipc_method_not_found_carries_jsonrpc_error.rs`** — call `foo.bar`, assert response is JSON-RPC error code `-32601`.
8. **`tests/ipc_socket_perms_are_0600.rs`** — boot `serve`, stat socket, assert mode bits.

`panops-protocol` unit tests:
- Serde round-trip for every type in `methods.rs` and every `IpcError` variant.
- `IpcError` `Unknown` deserialization: feed a JSON payload with `kind: "future_variant"`, assert it deserializes as `Unknown` (not error).
- `From<DomainErr>` mapping tests behind `#[cfg(feature = "domain-conversions")]`, one per domain variant (audit the table in §Architecture).

## Walking-skeleton implementation order (sketch)

The `superpowers:writing-plans` invocation produces the canonical step list; this is illustrative ordering for scope sanity:

1. New crate `panops-protocol` with empty lib + dependencies. Workspace-add. Build green.
2. `IpcError` enum + serde round-trip tests + `Unknown` forward-compat test.
3. `From<DomainErr>` impls behind `domain-conversions` feature + per-variant tests.
4. `methods.rs` request/response + event types + serde round-trip tests.
5. Refactor `GenaiLlm`: lazy shared runtime for CLI, `with_handle` for server. Existing CLI tests stay green.
6. Add `tokio = { features = ["rt-multi-thread", "macros", "signal"] }` and `jsonrpsee = "0.26"` to `panops-engine` deps.
7. `serve` subcommand: socket-lifecycle prelude (resolve path, probe live, unlink stale, bind, chmod, signal handlers). Test #1, #4, #5, #8.
8. `EngineServices` factory wiring with default factories (real adapters) + a `for_tests(...)` constructor for in-process injection.
9. Wire `meeting.list -> []`. Test #6, #7.
10. Wire `notes.generate` handler: spawn_blocking around `pipeline.generate`, post events on the broadcast channel. Test #2, #3.
11. `proto/ipc.md` documentation under `docs/proto/` per design §88.
12. File debt issues for the deferred surface (12 methods, 4 events, auth, persistence wiring, cancellation tokens).

Gate at end of slice: `cargo fmt && cargo clippy --workspace --all-targets --locked -- -D warnings` clean, `cargo test --workspace --locked` green, manual `panops-engine serve` then `websocat` event-subscription smoke check (logged in the session log, not in CI).

## Decisions (locked)

- **D1**: Server-with-CLI-subcommands. New `serve` subcommand alongside default mode and `notes`. Default mode keeps stdout JSON contract. Risk 5-A.
- **D2**: Two-runtime topology — Runtime A (jsonrpsee) and Runtime B (LLM HTTP) — with `spawn_blocking` between RPC handlers and the rayon-driven pipeline. Risk 5-B.
- **D3**: New crate `panops-protocol` hosts `IpcError` and the request/response types; `From<DomainErr>` impls live in this crate behind the `domain-conversions` feature flag (orphan rule satisfied; non-Rust consumers get a clean wire-types-only build). Risk 5-C corrected.
- **D4**: `IpcError` is `#[serde(tag = "kind", rename_all = "snake_case")]` with a unit `Unknown` variant marked `#[serde(other)]` for forward-compat. Adding new variants is non-breaking; renaming/removing is breaking.
- **D5**: `jsonrpsee 0.26` + low-level `serve_with_graceful_shutdown` API. Copy `examples/jsonrpsee_server_low_level_api.rs` as the UDS adapter template. WebSocket multiplexed on the same connection.
- **D6**: Event plane is a single `events.subscribe` WS subscription backed by a `tokio::sync::broadcast` channel. Late subscribers miss earlier events; replay deferred.
- **D7**: Stale-socket policy — probe with `connect`, refuse if live, unlink if dead. No `--force` flag.
- **D8**: Slice-05 socket perms are `0600` only. Token auth deferred.
- **D9**: `meeting.list` ships now with empty-array response; backed by SQLite when #17 lands.
- **D10**: Cancellation tokens through `LlmRequest` deferred — file as `severity:low` `area:ipc` `type:debt` issue at slice end.
- **D11**: Test client lives at `crates/panops-engine/tests/`; promote to a separate crate only when the Mac app actually consumes it.

## Done when

- `cargo test --workspace --locked` passes including all 8 IPC integration tests above.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.
- `panops-engine serve` binds the socket, accepts a `notes.generate` round-trip, and emits `job.done` over WS for the slice 04 fixture (manual smoke documented in session log).
- Crash recovery works: kill -9 the server, restart — second start succeeds (stale-socket cleanup).
- `proto/ipc.md` documents the two methods, two events, the `IpcError` taxonomy, the socket location, and the auth model (`0600` only).
- All deferred items are filed as GitHub issues with `type:debt area:ipc` (or `area:storage` for the SQLite-backed `meeting.list`).

## References

- Locked design: `docs/superpowers/specs/2026-04-30-panops-design.md` §83, §88, §178–217, §202–203.
- Brainstorm artifact: `docs/superpowers/specs/2026-05-02-slice-05-ipc-brainstorm.md`.
- jsonrpsee low-level API: [jsonrpsee_server_low_level_api.rs](https://github.com/paritytech/jsonrpsee/blob/master/examples/examples/jsonrpsee_server_low_level_api.rs).
- jsonrpsee subscriptions: [ws_pubsub_broadcast.rs](https://github.com/paritytech/jsonrpsee/blob/master/examples/examples/ws_pubsub_broadcast.rs).
- jsonrpsee Issue [#1264](https://github.com/paritytech/jsonrpsee/issues/1264) — TowerService WS close detection (subscribe via `sink.closed()` for per-connection lifecycle).
- jsonrpsee server docs: [jsonrpsee-server on docs.rs](https://docs.rs/jsonrpsee-server/latest/jsonrpsee_server/).
- PostHog post-mortem on rayon × tokio: [Untangling rayon and tokio](https://posthog.com/blog/untangling-rayon-and-tokio).
- Tokio async-blocking guidance: [Alice Ryhl — Async: what is blocking?](https://ryhl.io/blog/async-what-is-blocking/).
- Tokio discussion on `block_on` inside `spawn_blocking`: [tokio-rs/tokio#3717](https://github.com/tokio-rs/tokio/discussions/3717).
- Rust orphan-rule reference: [The Rust Reference — Trait Implementation Coherence](https://doc.rust-lang.org/reference/items/implementations.html#trait-implementation-coherence).
- Serde forward-compat: [Variant attributes — `#[serde(other)]`](https://serde.rs/variant-attrs.html).
- UDS cleanup background: [How to delete a Unix domain socket file when your application exits (2026 guide)](https://copyprogramming.com/howto/unix-domain-socket-not-closed-after-close).
