# Slice 05 — IPC layer (brainstorm artifact)

**Status:** brainstorm only. Not a spec. Per AGENTS.md, the next session must walk this through `superpowers:brainstorming` interactively, get user approval on each section, then write the locked spec at `docs/superpowers/specs/<latest>-slice-05-ipc-design.md` and follow with `superpowers:writing-plans`.

**Why this exists:** the user is offline and asked for a thorough async-reviewable artifact rather than interactive Q&A. This document collects the architectural decisions slice 05 needs, recommends one path for each with rationale, and surfaces the open questions the user must answer before coding.

**Scope context:** slice 05 is the LAST piece before the SwiftUI Mac shell (slice 06). Its only customer in this repo is the Mac app (and the test client). v0.1 polish (#35, #17, real-meeting calibration) is path A and is *not* this slice.

---

## 1. Scope split — what lands in slice 05 vs deferred

The locked design (`2026-04-30-panops-design.md` §178–217) lists 14 control methods and 6 event types. Walking-skeleton discipline says we ship the smallest cross-cutting path end-to-end, then the second slice fills in. The split:

### In slice 05 (the walking skeleton)

| Surface | Reason |
|---|---|
| `panops-protocol` crate (new) | DIP gate — domain crate stays transport-free. Hosts request/response types + `IpcError`. |
| Unix socket bind + stale-socket cleanup at `~/Library/Application Support/panops/engine.sock` | Without this nothing else is reachable. |
| `panops-engine serve` subcommand (new) | Resolves Risk 5-A; the existing `notes` and default-mode CLI behaviour remain unchanged. |
| **One** control method: `notes.generate(audio_path, dialect?, llm_provider?) -> JobId` | Smallest end-to-end shape. Exercises ASR + diar + LLM + exporter — every existing port. |
| **One** control method: `meeting.list() -> []` (returns `[]` until #17 lands) | Hello-world for the Mac app's launch screen. Forces the request/response/error shape to settle. |
| **One** event: `job.done { job_id, result: { primary_file, assets } }` | Smallest end-to-end event shape. |
| **One** event: `job.error { job_id, error: { kind, message } }` | Same shape carries error transport (Risk 5-C). |
| WebSocket event plane on the same socket (HTTP upgrade) | Future events all flow here; if it doesn't work for `job.done` it won't work for `asr.partial` later. |
| Rust integration test client | AGENTS.md mandates this: spawn `panops-engine serve` in-process or as child, drive a real round-trip. |
| Stale-socket-on-crash test | Known jsonrpsee/UDS gotcha; cheap to test once. |

### Deferred to a follow-up slice (file as `type:debt area:ipc` issues at slice end)

- Remaining 12 control methods (`meeting.start/stop/get/delete/set_language`, `asr.post_pass/cancel`, `notes.export`, `llm.probe/providers/test`, `settings.*`).
- Live-capture events (`asr.partial`, `asr.final`, `screenshot`, `job.progress`).
- `Storage` port + SQLite persistence (#17). The walking skeleton runs notes generation directly from a wav path — no DB read/write yet. **`meeting.list` returns `[]` deliberately.** When #17 lands it gets backed by SQLite without reshaping the IPC surface.
- `LlmProviderProbe` port (`llm.probe` requires it; defer per "one port at a time" rule).
- Auth on the socket (filesystem perms `0600` are slice 05's only mechanism; no token auth).
- WebSocket reconnection / replay / event backpressure.

**Why this split is the smallest viable slice:** it touches the binary boundary (Risk 5-A), the runtime (Risk 5-B), and the error transport (Risk 5-C) — the three load-bearing decisions. If any is wrong, we'll know before writing the other 12 methods.

---

## 2. Architectural decisions

### Risk 5-A — stdout/IPC fd contract

**Problem:** `panops-engine` default mode does `println!("{json}")` for the transcript JSON (`crates/panops-engine/src/main.rs:163`). A long-lived JSON-RPC server cannot share that stdout contract.

**Options considered:**

1. **Server-with-CLI-subcommands** (`panops-engine serve | notes | transcribe`). One binary, dispatch in `main()`. Matches AGENTS.md's "one binary" framing.
2. **Server spawns CLI as child.** Two binaries; server shells out to a CLI helper for actual work. Simpler isolation but doubles the install footprint and complicates dylib loading (whisper-rs, sherpa).
3. **Server-only binary; drop the CLI.** The dev-CLI was always "not the product UX" (header comment in `main.rs:1`). Could be retired entirely.

**Recommendation: Option 1 (server-with-subcommands).**

- The default mode already became a subcommand-shaped thing the moment we added `notes` (slice 04). One more `serve` subcommand is consistent.
- One binary = one dylib load, one model cache, one place for tracing init.
- Keeps the CLI alive for CI smoke tests and dev iteration without the Mac app.
- Default mode (no subcommand) keeps its current contract: stdout is the JSON transcript. `serve` mode never writes to stdout (logs go to stderr via tracing, which already excludes ANSI per `main.rs:144`).

**Implication:** the existing default-mode `println!` at `:163` stays. Slice 05 adds `Cmd::Serve { socket: Option<PathBuf> }` next to `Cmd::Notes`. No fd contract collision because `serve` never prints to stdout.

**Open question for the user:** do you also want to rename default-mode into an explicit `transcribe` subcommand for symmetry, or keep the current shape (default = transcribe) for backwards-compat with shell pipelines? Recommendation: keep default as-is to avoid churning slice 02–04 tests; revisit at v0.1.

---

### Risk 5-B — rayon × tokio runtime collision

**Problem:**
- `panops-core/src/notes/pipeline.rs:60` uses `rayon::par_iter` for parallel section LLM calls.
- `panops-portable/src/genai_llm.rs:13,63` owns `Arc<Runtime>` and `block_on`s every call.
- A jsonrpsee server runs its own tokio runtime. Three failure modes if naïvely composed: (a) rayon worker threads blocking inside `block_on` starve the runtime they're called from; (b) per-`GenaiLlm`-instance `Runtime::new()` multiplies thread count under concurrent requests; (c) cancellation drops a runtime mid-`block_on`.

**Options considered:**

1. **Make `LlmProvider::complete` `async fn`** and replace rayon with `tokio::task::JoinSet`. Cleanest end-state but trait churn touches every existing adapter (`GenaiLlm`, `MockLlm`, `FakeNotesExporter` is fine — only LLM matters). Forces all of `pipeline.rs` to become async.
2. **Keep sync trait, route every pipeline call through `tokio::task::spawn_blocking`** against a single shared blocking-thread pool. The IPC server calls `spawn_blocking(move || pipeline.generate(input))`; pipeline keeps using rayon internally; `GenaiLlm` can drop its private runtime and reuse the server's via a thin shim or a `OnceLock<Runtime>`.
3. **Status quo + put the IPC server outside any pipeline thread.** Send work to a channel, have a dedicated worker thread (or rayon pool) drain it, return results via oneshot. Avoids both async sprawl AND spawn_blocking but reinvents Tower's job pattern.

**Recommendation: Option 2 (sync trait + spawn_blocking + shared runtime).**

- Trait churn is contained: the only public-facing change is dropping `Arc<Runtime>` from `GenaiLlm` and accepting a `&tokio::runtime::Handle` (or `OnceLock<Runtime>` lazy default for non-server callers like the CLI).
- Pipeline stays synchronous → existing 47 panops-core tests, 38 panops-portable tests, the conformance harness, all stay valid.
- rayon for *intra-job parallelism* (parallel section LLM calls within one `notes.generate`) plus tokio for *inter-job concurrency* (multiple in-flight RPCs) is a clean separation; the pools don't share threads.
- One shared `Runtime` for `GenaiLlm` removes the per-instance multiplication (#5-B-c).
- spawn_blocking gives clean cancellation semantics: drop the JoinHandle and the request future returns; the pipeline task continues to completion (rayon doesn't cancel) but its `LlmError::Cancelled` path (already exists, `panops-core/src/llm.rs:44`) lights up via a cancellation token threaded into `LlmRequest` — defer that to a follow-up issue.

**Implication for slice 05:**
- Add `tokio = { version = "1", features = ["rt-multi-thread", "macros"] }` to `panops-engine` only (not core, not portable).
- `GenaiLlm` API change: `GenaiLlm::new(model)` keeps a default lazy `Runtime` (CLI use); `GenaiLlm::with_handle(model, handle)` for server use.
- Wrap `pipeline.generate(input)` calls in `spawn_blocking` inside the `notes.generate` RPC handler.
- File a `type:debt severity:medium area:ipc` issue: "thread `CancellationToken` through `LlmRequest` for IPC cancellation".

**Open question for the user:** are you OK with `GenaiLlm::new` keeping an internal `Runtime` (for CLI) AND `with_handle` (for server)? Or prefer to move the runtime ownership out of the adapter entirely and inject it from the binary in both cases? Recommendation: keep the dual API. CLI users (and the regen test) shouldn't need to know about runtimes.

---

### Risk 5-C — error transport over IPC

**Problem:** `NotesError`, `AsrError`, `DiarError`, `LlmError` derive `Debug + thiserror::Error` only. The `job.error` event payload (design §215) needs them serialisable.

**Options considered:**

1. **Add `serde::{Serialize, Deserialize}` to all domain errors directly.** Minimal boilerplate. But adds a transport dep to the domain crate (panops-core gets `serde` derive on errors that thus far are pure thiserror). Marginal violation of DIP since the domain crate doesn't *need* serde for errors; only the IPC layer does.
2. **Define `IpcError` in `panops-protocol` with `From<NotesError>` etc.** Cleaner hex; domain stays transport-free. Costs ~1 enum + 4 `From` impls (~50 lines).
3. **String-only error transport: `job.error { error: String }`.** Lossy. The Mac app can't distinguish "audio not found" from "LLM provider unavailable" without parsing prose.

**Recommendation: Option 2 (IpcError in panops-protocol).**

- Domain crate stays serde-free for error types — important because the AGENTS.md DIP rule is the one architectural invariant most likely to slowly erode.
- The `From` impls collapse domain-error variants into a small surface the UI actually needs. Suggested taxonomy:
  ```rust
  // crates/panops-protocol/src/error.rs
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
  }
  ```
- `panops-protocol` already exists in slice plans (design §83). Slice 05 is when it's born.
- Fingerprint of the wire format: external string tag + UTF-8 message. Forward-compatible (new `kind` values can land without breaking old clients; clients fall back to `Internal`).

**Implication for slice 05:**
- New crate `crates/panops-protocol/` with two modules: `methods` (request/response types) and `error` (`IpcError`).
- `panops-protocol` depends on `serde`, `thiserror`. Does NOT depend on `panops-core` — keeps the protocol crate buildable from a Mac/Swift codegen target later. The `From<NotesError>` impls live in `panops-engine` (the only thing that bridges the two), not in protocol.
- All 4 domain errors get a `From<X> for IpcError` in engine-side glue.

**Open question for the user:** should `IpcError` carry an optional structured `details: serde_json::Value` field (LLM stack trace, audio duration when invalid, etc.)? Adds debugging surface; risks leaking info the UI can't render. Recommendation: ship without it; add when the Mac app actually needs more.

---

## 3. Dependency choices

### JSON-RPC + WebSocket transport

**Pick: `jsonrpsee` (server + ws + http + macros) + manual UDS bind.**

- Async/await native, hyper v1 stack (the modern Rust state-of-the-art for 2025–2026).
- Built-in WebSocket upgrade on the same listener — eliminates the "control + event on separate sockets" complexity.
- Macros generate request/response types from a trait; pairs cleanly with `panops-protocol`'s shared types.
- Active maintenance (Parity / Substrate ecosystem).
- Caveats:
  - jsonrpsee binds to TCP sockets natively. UDS support requires constructing the listener manually with `tokio::net::UnixListener` and feeding accepted streams to the jsonrpsee server's `connect()` method. Documented but not turnkey.
  - **Stale-socket gotcha:** UDS files persist after a crash. Slice 05 must check for and `unlink()` the path before `bind()`, with one safety: refuse to delete if a live process is listening (try `connect()`; if it succeeds, the user already has an engine running — exit with a clear error, not a silent steal).

**Alternatives considered and rejected:**
- `jsonrpc-core` / `jsonrpc-pubsub` (Parity, predecessor): unmaintained, sync-first.
- Raw `tonic` / gRPC: heavier, requires `.proto` files, the design spec specifies JSON-RPC + WS so a swap would be a design-spec change.
- Hand-rolled JSON-RPC over `tokio::net::UnixStream`: doable but reinvents framing, batching, error spec compliance. Skip.
- `tarpc`: tight Rust↔Rust ergonomics but not JSON-RPC 2.0 wire-compatible — Swift client would need a custom codec.

### Async runtime
`tokio` with `rt-multi-thread`. No `async-std`, no `smol` — jsonrpsee assumes tokio.

### WebSocket
Comes from jsonrpsee's `ws-server`. No standalone `tokio-tungstenite` needed for slice 05.

### Serialization
`serde` + `serde_json` already in the workspace. No new deps.

### Test client
`jsonrpsee-ws-client` for the integration test. Same crate family, same version.

**Rough Cargo.toml budget for slice 05** (subject to MSRV check):
- `panops-protocol`: `serde`, `serde_json`, `thiserror`. Three deps.
- `panops-engine` (new deps only): `tokio` (multi-thread), `jsonrpsee = { version, features = ["server", "ws-server", "macros"] }`, `tracing-futures` (instrument async spans). ~3 new deps.
- Dev-deps: `jsonrpsee-ws-client` for the test client.

**Open question for the user:** OK with jsonrpsee adding ~80 transitive crates? An alternative is hand-rolling JSON-RPC framing in ~300 lines (one-line-per-request over UDS, no batching, no WS — defer events to slice 05.5). Recommendation: take jsonrpsee. Slice 06 needs WS for live-capture events; reinventing it twice is the wrong trade.

---

## 4. Test surface

AGENTS.md: "Live ASR has a fixture set [...] Adapters must hit a WER threshold." Slice 05 doesn't add an adapter, so it doesn't take the conformance-per-port rule head-on. Instead its test surface is a Rust integration test client per the slice sequence.

**Required tests (gate slice 05 PR):**

1. **`tests/ipc/server_starts_and_binds.rs`** — spin up `panops-engine serve --socket <tmp>` as a child, assert socket file appears, assert connect succeeds, send shutdown, assert socket cleaned up.
2. **`tests/ipc/notes_generate_round_trip.rs`** — `notes.generate` over JSON-RPC against the multi_speaker_60s fixture (ASR via `PANOPS_FAKE_ASR=1`, LLM via injected `MockLlm` — see §5 below for the injection mechanism), assert `job.done` event arrives over WS with the expected `primary_file` and that the file exists. End-to-end: fixture → real pipeline → real exporter → real WS event.
3. **`tests/ipc/job_error_carries_kind.rs`** — call `notes.generate` with a non-existent audio path, assert `job.error { kind: "input_not_found", message }` arrives.
4. **`tests/ipc/stale_socket_is_cleaned.rs`** — pre-create a stale socket file (no live listener), assert `serve` removes it and binds successfully.
5. **`tests/ipc/refuses_to_steal_live_socket.rs`** — start one server, start a second on the same path, assert second exits non-zero with a clear "engine already running" message.
6. **`tests/ipc/meeting_list_returns_empty.rs`** — sanity-check the second control method shape; until #17 lands this just confirms the response framing.
7. **`tests/ipc/method_not_found_carries_jsonrpc_error.rs`** — call an unknown method (`foo.bar`), assert the response is a proper JSON-RPC `-32601` error, not a panic / dropped connection.

**Conformance update:**
- `panops-protocol` gets serde round-trip tests for every type (already the convention; see `panops-core/src/llm.rs:65`).
- `IpcError` round-trips through serde without losing the `kind` tag — one test per variant.

**Don't test in slice 05:**
- Real network LLMs over IPC (gated behind `PANOPS_REAL_*` envs in slice 04 anyway).
- Concurrent-request load (defer to follow-up).
- WS reconnection (deferred above).

**Open question for the user:** does the test client live under `crates/panops-engine/tests/` or in a new `crates/panops-test-client/` (binary that the Mac app's developers can also use as a reference)? Recommendation: start under `panops-engine/tests/`; promote to a separate crate only when the Mac app actually consumes it.

---

## 5. MockLlm injection across the IPC boundary

Slice 05's integration tests need to drive `notes.generate` with a deterministic LLM. Today the pipeline takes `&dyn LlmProvider` directly. Across an IPC boundary, the test can't pass a Rust trait object.

**Options:**

1. **Per-test sidecar binary**: a separate `panops-engine-test` binary with a hard-coded `MockLlm` table. Heavyweight.
2. **`PANOPS_LLM_PROVIDER=mock` mode** that loads canned responses from a JSON file at a path given by `PANOPS_MOCK_LLM_FIXTURE`. Mirrors the existing `PANOPS_FAKE_ASR=1` env pattern.
3. **In-process server**: launch the server inside the test process via `jsonrpsee::server::ServerBuilder::build`, not as a child. The test can hold a `MockLlm` directly. Removes the env-var workaround entirely.

**Recommendation: Option 3 (in-process server) for unit-style tests, Option 2 for the smoke test that verifies the `serve` binary actually works as a child.**

- Most IPC tests are about wire shape, not about subprocess lifecycle. In-process is faster and avoids "is the binary built" flakiness.
- The smoke test (`server_starts_and_binds.rs`) is the one place where we DO care about the binary; the `mock` env path keeps it deterministic.
- `MockLlm` already exists at `crates/panops-core/src/conformance/fakes.rs:108` — no new fake needed.

**Implication:** slice 05 introduces `panops_engine::server::ServerHandle` (or similar) as a public test surface — `pub async fn start_in_process(deps: Deps) -> ServerHandle`. The `Deps` struct accepts `Arc<dyn LlmProvider>`, an ASR factory, etc. CLI `serve` calls the same `start_in_process` with default deps.

**Open question for the user:** OK with `Deps` (ASR factory + LLM factory + diar factory + exporter factory) becoming the slice 05 wiring point? It's a small DI container, basically. Recommendation: yes, but call it `EngineServices` rather than `Deps`. Document its layout in the spec so slice 06 doesn't have to guess.

---

## 6. Walking-skeleton steps (slice 05 plan sketch)

Roughly the order `superpowers:writing-plans` should produce. NOT a final plan — illustrative, to make the slice tractable in scope.

1. **Create `panops-protocol` crate** with empty `lib.rs` + `serde` + `thiserror`. Workspace-add. Build green.
2. **Add `IpcError` enum** + serde round-trip tests for every variant. No domain integration yet.
3. **Add request/response types for `notes.generate` and `meeting.list`** (`NotesGenerateParams`, `NotesGenerateResult`, `MeetingSummary`). Serde round-trip tests.
4. **Add `From<NotesError|AsrError|DiarError|LlmError> for IpcError`** in `panops-engine` (NOT in protocol crate). Tests covering each domain variant.
5. **Refactor `GenaiLlm`** to accept an optional `tokio::runtime::Handle`. Existing CLI users keep working via the lazy default. Test: `GenaiLlm::with_handle` shares a runtime across two instances without spawning extra threads. (May need a follow-up debt issue if the genai client itself spawns its own pool — investigate, don't pre-fix.)
6. **Add `panops-engine serve` subcommand** that binds a UDS, sets `0600` perms, registers a no-op handler, exits cleanly on SIGINT. Integration test #1 + #4 + #5 above.
7. **Wire `notes.generate` handler** that calls the existing pipeline via `tokio::task::spawn_blocking`. Integration test #2.
8. **Wire `job.error` event** for the audio-not-found path. Integration test #3.
9. **Wire `meeting.list -> []` stub.** Integration test #6.
10. **Add `method_not_found` test** (#7) — should pass for free with jsonrpsee but is a regression gate.
11. **Document the protocol** at `docs/proto/ipc.md` — minimal: socket location, method list (current = 2), event list (current = 2), error taxonomy. Spec §88 calls for this.
12. **File debt issues** for the deferred 12 methods + cancellation token + auth.

Gate at the end: `cargo test --workspace --locked` green + manual one-liner `panops-engine serve` then `wscat`/`websocat` sanity-check the socket.

---

## 7. Out of scope (file as `type:debt area:ipc` or `area:storage` issues at slice end)

- `meeting.start/stop/get/delete/set_language` — needs #17 (storage).
- `asr.post_pass/cancel` — needs cancellation tokens.
- `notes.export` — pipeline already exposes the function; trivial after slice 05's wiring exists.
- `llm.probe/providers/test` — needs the `LlmProviderProbe` port (design §202–203).
- `settings.get/set` — needs storage.
- `asr.partial`, `asr.final`, `screenshot`, `job.progress` events — need live capture (slice 07).
- WebSocket reconnection / replay buffer — Mac app can re-issue calls until evidence says otherwise.
- Auth tokens beyond `0600` filesystem perms.
- Multi-client fan-out of events (one engine, many subscribers) — unclear if Mac app even needs it; `0600` already pins to one user.
- Cross-platform UDS (Linux is fine, Windows would need named pipes — design is mac-first).

---

## 8. Open questions surfaced for the user's next session

In the order they'd come up in a brainstorm:

1. **CLI shape:** keep default-mode (no subcommand) as `transcribe`, or rename to an explicit subcommand for symmetry? (§2 / Risk 5-A)
2. **Runtime ownership:** `GenaiLlm::new` keeps an internal lazy `Runtime` for CLI users; server uses `with_handle`. Acceptable, or move runtime ownership entirely to the binary? (§2 / Risk 5-B)
3. **`IpcError` variants:** taxonomy proposed (`InputNotFound`, `InvalidInput`, `ProviderUnavailable`, `Internal`, `Cancelled`). Add `details: serde_json::Value`? (§2 / Risk 5-C)
4. **jsonrpsee dep weight:** OK with ~80 transitive crates, or hand-roll a 300-line JSON-RPC over UDS for slice 05 and pull jsonrpsee in at slice 06 when WS becomes load-bearing? (§3)
5. **Test client crate location:** `crates/panops-engine/tests/` (recommended) or new `crates/panops-test-client/`? (§4)
6. **`EngineServices` (DI container):** acceptable name and shape, or different name? Should it live in `panops-engine` only, or be public from `panops-protocol` so the Mac app can build a Rust-side mock too? (§5)
7. **Stale-socket policy:** "if the socket is live, refuse and exit; if dead, unlink and bind." Strict enough? Should `serve --force` exist for ops? Recommendation: no. (§3)
8. **Socket location override:** add `--socket <path>` flag to `serve` for tests, OR use `PANOPS_SOCKET` env? Recommendation: flag for tests, env discouraged (matches "no env vars for user config" rule). (§3 / §6)

---

## 9. Verification scaffolding

This brainstorm's claims can be checked before committing the spec:

```bash
# Sync tracing decision: are there leftover Arc<Runtime>s elsewhere?
rg 'Arc<Runtime>|Runtime::new' crates/

# Sync rayon decision: count rayon sites the IPC layer must wrap.
rg 'rayon::|par_iter|par_bridge' crates/panops-core/

# Domain errors that need IpcError mapping.
rg '#\[derive\(.*Error.*\)\]' crates/panops-core/src/

# jsonrpsee: confirm UDS pattern in current docs.
# https://docs.rs/jsonrpsee-server/latest/jsonrpsee_server/struct.ServerBuilder.html
# https://github.com/paritytech/jsonrpsee
```

---

## 10. What this artifact is NOT

- Not a slice spec. The next session must walk this through `superpowers:brainstorming` interactively to satisfy the user-approval gate AGENTS.md requires before locking architecture.
- Not a plan. `superpowers:writing-plans` runs after the spec is locked.
- Not exhaustive. The IPC plane has surface — auth, sandbox, audit logging, structured cancellation — that v0.1 doesn't need but v1.0 will. Those decisions defer until they're load-bearing.

---

## Appendix — references

- Locked design: `docs/superpowers/specs/2026-04-30-panops-design.md` §178–217 (IPC), §83 (panops-protocol crate), §88 (`proto/ipc.md`), §202–203 (`LlmProviderProbe`).
- Project review identifying 5-A/5-B/5-C: `~/.claude/plans/lets-go-debt-first-shimmying-fairy.md` §4.
- AGENTS.md "Execution discipline" — one trait + one real + one fake; one slice = one PR.
- Web research (April 2026):
  - jsonrpsee — https://github.com/paritytech/jsonrpsee
  - jsonrpsee-server ServerBuilder — https://docs.rs/jsonrpsee-server/
  - Rust forum: serialisation/RPC crates for Unix pipes/sockets — https://users.rust-lang.org/t/serialisation-rpc-crates-for-unix-pipes-and-or-unix-domain-sockets/113995
- Code anchors verified at brainstorm time:
  - `crates/panops-engine/src/main.rs:163` — println contract
  - `crates/panops-core/src/notes/pipeline.rs:60` — rayon par_iter
  - `crates/panops-portable/src/genai_llm.rs:13,63` — private Runtime + block_on
  - `crates/panops-core/src/llm.rs:34–46` — LlmError variants (Cancelled exists, unused so far)
  - `crates/panops-core/src/conformance/fakes.rs:108` — MockLlm location for IPC test injection
