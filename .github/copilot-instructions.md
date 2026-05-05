# Copilot review instructions — panops

## Project shape

panops is a local-first macOS recorder + screenshot-anchored meeting notes generator. Hexagonal Rust core: `crates/panops-core` is platform-agnostic; `crates/panops-portable` holds shared adapters; `crates/panops-mac` is `#[cfg(target_os="macos")]`-gated (planned, slice 06+); `crates/panops-engine` is the binary; `crates/panops-protocol` is IPC types-only. SwiftUI shell at `apps/Panops/` plus sidecars under `apps/panops-asr-mac/` and `apps/panops-llm-mac/` land in slice 06 (planned, not present).

Authoritative project rules, in priority order: `docs/north-star.md` > active slice spec under `docs/superpowers/specs/` > `docs/superpowers/specs/2026-04-30-panops-design.md` > active slice plan under `docs/superpowers/plans/` > `AGENTS.md`. The GitHub Project board (https://github.com/users/vfmatzkin/projects/1) wins for in-flight state (status, ownership, blockers). Architecture is locked; flag drift, don't tolerate it.

## Review priorities (highest first)

1. **Correctness under failure.** Async/concurrency code must handle: panics inside `spawn_blocking`/`spawn` (caught, not swallowed); SIGTERM mid-init; canceled futures dropping shared state; `OnceLock`/`OnceCell` reaching a terminal state on every path (success OR error OR panic — never permanently `None` if a handler can observe it).
2. **Hex-arch invariants.** `panops-core` must not depend on `tokio`, `reqwest`, `whisper-rs`, `sherpa-rs`, or any platform crate. Ports (`AsrProvider`, `Diarizer`, `LlmProvider`, `NotesExporter`) live in `panops-core`; real adapters live in `panops-portable` or `panops-mac`. Domain errors (`AsrError`, `DiarError`, `LlmError`, `NotesError`) must not derive `serde::Serialize` — IPC transport types in `panops-protocol` convert from them.
3. **Port introduction discipline.** A new port trait must land with at least one real adapter and one fake (`panops-core::conformance::fakes`) plus a conformance harness fn that both pass. Additional real adapters for an existing port are fine. Reject pre-traited interfaces with no consumer.
4. **No telemetry, no env vars for user config.** Anything that phones home is a blocker (per AGENTS.md "NEVER phone home"). Env vars are last-resort; primary config flows through the IPC API or auto-detection.
5. **Stdout contract.** `panops-engine` default-mode prints exactly one JSON object to stdout (final result via `println!`). Errors go to stderr; the engine entry point uses `eprintln!("error: {msg}")` and that pattern is canonical for the CLI error path. Diagnostic output in production code must use `tracing::*` macros, never `println!`/`eprintln!` outside the CLI's error-and-final-output paths.

## Concrete things to flag

- `unwrap()` / `expect()` on user input, IPC params, audio/video bytes, or LLM responses. Tests may unwrap.
- Missing `tokio::task::spawn_blocking` around sync model-load or rayon calls when invoked from an async handler.
- `process::exit` without first dropping owned tokio runtimes, OR dropped runtimes that hold non-cancellable `spawn_blocking` work (leaked file descriptors / mmap regions are acceptable; lost ack writes are not).
- `Arc<OnceLock<...>>` patterns where a panic in the writer path leaves the slot `None` forever. Always wrap initializing closures in `std::panic::catch_unwind` and convert panic payloads to `Err`.
- New methods on `IpcServer` that don't appear in `docs/proto/ipc.md`. That doc is the wire contract; keep both in sync.
- Markdown writer code that produces something other than the locked dialect set (`NotionEnhanced`, `Basic`). New dialects require a spec amendment.
- Storage code that bypasses the per-meeting SQLite at `~/Library/Application Support/panops/meetings/<uuid>/meeting.db` or the registry at `~/Library/Application Support/panops/panops.db`.
- "deferred / out-of-scope / follow-up" comments in source. Per AGENTS.md "Debt rule" these must be GitHub issues, not buried comments.
- Workflow YAML changes that expose `secrets.*` to PRs from forks. The underlying rule: jobs that read secrets must gate on something that proves the run is trusted (e.g. `github.event.pull_request.head.repo.full_name == github.repository`, `github.actor == 'dependabot[bot]'`, or running under `pull_request_target` with explicit allowlists). Flag any new workflow that reads secrets without such a gate; do not flag workflows that already use a different valid gate.

## Things to NOT flag

- `unwrap()` / `expect()` inside `#[cfg(test)]`, `tests/`, `benches/`, or doc-tests.
- Single-line shell-style commit messages without trailing punctuation (project convention).
- Use of `tracing::*` instead of structured logging crates (intentional choice).
- Long `match` arms on `IpcError` in `panops-protocol` — the kind tags are stable wire shape, exhaustiveness is the goal.
- Lack of `#[non_exhaustive]` on internal enums; the project favours explicit match exhaustiveness.
- File-scoped `#[allow(...)]` is fine in test modules; flag only in production code.

## Severity calibration

- **Blocker** (request changes): hex-arch violation, panic-leaving-None, telemetry, missing conformance test for a new port, broken stdout contract, secrets in workflows.
- **Suggestion** (comment): style nits, naming, dedup, doc improvements.
- **Note** (FYI): observations the author should know but isn't expected to act on in-PR.

## Style + Rust conventions

- `cargo fmt` is canonical. Don't request formatting changes — `rustfmt` runs in CI.
- Prefer `tracing::info!/warn!/error!` with structured fields over message-only logs.
- Function visibility: `pub` only at crate boundaries; `pub(crate)` / `pub(super)` for internal sharing.
- Prefer `tokio::sync::watch` for terminal-state signals (shutdown), `mpsc` for streams, `broadcast` for fan-out. Avoid `Notify` for shared state — it carries no value and is easy to lose wakeups against; use `watch` instead.
- Tests that hit a real Whisper or Sherpa model must be gated by a `PANOPS_*` env var so CI defaults stay fast.

## When uncertain

If a change appears to violate `AGENTS.md` or the design spec but the PR description says it's intentional, surface the conflict explicitly — don't silently approve or silently block.
