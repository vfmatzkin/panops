# Slice 06 — SQLite storage + meeting lifecycle: design

**Status:** locked. Amendments require a maintainer decision recorded inline with a date stamp.
**Closes:** #17 (no SQLite persistence), #75 (IPC: implement `meeting.*` control methods).
**Brainstorm:** `docs/superpowers/specs/2026-05-05-slice-06-storage-brainstorm.md` (2026-05-05).
**Slice tracking issue:** TBD (open with `gh issue create --label type:feature --milestone slice-06-storage`).

## Why this shape

Four load-bearing decisions, each with an alternative considered and rejected:

1. **Storage is one trait, one real impl, one fake.** Per AGENTS.md "MUST introduce one trait at a time". `Storage` lives in `panops-core`; `RusqliteStorage` lives in `panops-portable` (per the locked design doc: "SQLite is portable already"); `InMemoryStorage` fake lives in `panops-core::conformance`. A conformance harness exercises both. Rejected: pre-trait for hypothetical `PostgresStorage` / `IcloudStorage`.

2. **Single registry DB now; per-meeting DB deferred to Anchor B.** One `~/Library/Application Support/panops/panops.db` with `meeting + note` tables. The locked design doc spec at `docs/superpowers/specs/2026-04-30-panops-design.md:222-252` calls for a per-meeting `meeting.db` containing `segment + speaker + screenshot + job + note + meeting`. Slice 06 has none of `segment / speaker / screenshot / job` data to put there yet — those tables would sit empty, and `meeting.list` would pay N+1 opens for nothing. The per-meeting DB lands when live capture (Anchor B) creates content for it. **This is a deliberate spec amendment**, not silent drift.

3. **`meeting.start` / `meeting.stop` ship as data-plane scaffolding, not capture-coupled methods.** Without ScreenCaptureKit (Anchor B), `start` creates the row + meeting directory + `screenshots/` + sets `started_at`; `stop` writes `ended_at` and computes `duration_ms`. No capture wiring. Anchor A (Mac shell) calls them when the user clicks record/stop; Anchor B wires them to real capture. Rejected: defer `start/stop` entirely to Anchor B (would leave the lifecycle surface inconsistent across slice 06 and Anchor A).

4. **`notes.generate` extends, doesn't break.** Adds optional `meeting_id: Option<String>` field to `NotesGenerateParams`. If absent, the handler auto-creates a meeting and writes into the canonical `meetings/<uuid>/` layout. If present, the handler attaches notes to the existing meeting. Existing IPC clients (the engine's own integration tests, future Mac shell early scaffolding) keep working without modification. Rejected: a separate `meeting.create_from_audio + notes.generate(meeting_id)` two-step (cleaner conceptually, but breaks the existing single-call CLI flow).

## What the maintainer actually said

Per the May 2 alignment audit's recommendation #2 ("add a 'what the user actually said' section to each slice spec"), here are the maintainer's verbatim decisions captured during the 2026-05-05 brainstorm. Anything in this spec that isn't in these quotes is an assistant default, marked "(assistant default)" inline.

> **Question:** Which trajectory item should the next slice cover?
> **Maintainer:** "#17 SQLite persistence (Recommended)"

> **Question:** How fat should slice 06 be?
> **Maintainer:** "Fat: full meeting lifecycle (#17 + #75)"

> **Question:** What should `meeting.start` / `meeting.stop` mean in slice 06, before live capture exists?
> **Maintainer:** "Data-plane scaffolding (Recommended)"

> **Question:** Per-spec split now, or single-DB for slice 06 with split deferred to Anchor B?
> **Maintainer:** "Single-DB now, split when needed (Recommended)" — selected with the preview showing the schema sketch in §Architecture below.

Everything else (concurrency model, error taxonomy, schema versioning approach, CLI flag shape, table column choices, RusqliteStorage internal structure, default `Mutex<Connection>` over a connection pool) is an assistant default the maintainer accepted by approving this spec.

## Scope (in this slice)

- New port `Storage` in `panops-core::storage` with associated domain types (`Meeting`, `MeetingDraft`, `Note`, `NoteDraft`) and `StorageError`.
- New conformance harness `panops-core::storage::conformance::storage_conformance(adapter)` that runs the same suite against any `Storage` impl (matching the existing `asr_conformance` / `notes_exporter_conformance` patterns).
- New fake adapter `InMemoryStorage` in `panops-core::conformance::fakes` (matches the existing `MockLlm` / `FakeNotesExporter` pattern).
- New `rusqlite` crate dep in `panops-portable` with `bundled` feature.
- New real adapter `RusqliteStorage` in `panops-portable` over a single SQLite file at the data-dir root.
- DB schema version 1 with two tables (`meeting`, `note`) — full DDL in §Architecture.
- `EngineServices` gains `Arc<dyn Storage>` and `data_dir: PathBuf` fields (both direct, NOT in the `heavy` OnceLock — SQLite open is cheap); `serve` constructs eagerly at startup with `--data-dir` (defaults to canonical macOS path).
- IPC methods (extend `crates/panops-protocol`). Method names below are the Rust trait names; the on-the-wire names carry the `ipc.` namespace prefix (e.g., `meeting.list` → wire method `ipc.meeting.list`) per the existing `#[rpc(server, namespace = "ipc", namespace_separator = ".")]` declaration on the `Ipc` trait at `crates/panops-engine/src/server/handlers.rs`.
  - `meeting.list() -> Vec<MeetingSummary>` — replace stub at `crates/panops-engine/src/server/handlers.rs:116-120` with real impl.
  - `meeting.start(MeetingConfig) -> meeting_id` — data-plane only.
  - `meeting.stop(id) -> Meeting`.
  - `meeting.get(id) -> Meeting`.
  - `meeting.delete(id) -> ()` — registry row first, then `rm -rf meetings/<id>/`.
  - `meeting.set_language(id, lang)`.
  - `notes.generate` extended with optional `meeting_id` (auto-create on `None`).
- New CLI flag `--data-dir <path>` on `panops-engine` (no env var; per drift §1).
- Existing CLI `panops notes <wav>` continues writing notes next to audio AND newly registers the meeting in the registry. CLI default mode (`panops <wav>` → JSON to stdout) does NOT register — it produces no notes artifact, and a meeting-without-notes row is misleading state for the Mac shell and future UIs that list meetings expecting notes. **Amended 2026-05-05** from the original brainstorm which had both modes registering; ratified by maintainer post-implementation.
- Protocol doc at `docs/proto/ipc.md` updated: new methods, new `meeting.*` shape, updated "What's NOT shipped" list.

## Out of scope (defer; file as `type:debt` issues at slice end)

- Per-meeting `meeting.db` (split deferred to Anchor B). Tables `segment`, `speaker`, `screenshot`, `job` remain unborn until that slice.
- Schema migrations beyond version 1. PRAGMA `user_version` mismatch hard-errors with `StorageError::SchemaMismatch`. v0.2 will pick a migration framework.
- Meeting-level full-text search across notes content.
- WebSocket events for storage mutations (e.g., `meeting.created`, `meeting.deleted`). Slice 06 returns the new `Meeting` from the request handler; subscribers don't get push updates yet.
- Cross-meeting transactions / atomic multi-meeting operations.
- WAL mode + per-task connections (single `Arc<Mutex<Connection>>` is single-user-fine).
- Cleanup of orphan `meetings/<uuid>/` directories whose registry rows were lost (e.g., manual `rm` of `panops.db`). Out-of-band recovery; document in protocol doc as a known limitation.
- Token-based auth on IPC (filed as #83).
- Filesystem encryption (relies on FileVault).
- Cloud sync / multi-device (architectural future).

## Architecture

### Port — `panops-core::storage::Storage`

```rust
// crates/panops-core/src/storage/mod.rs

pub trait Storage: Send + Sync {
    fn create_meeting(&self, draft: MeetingDraft) -> Result<Meeting, StorageError>;
    fn get_meeting(&self, id: &str) -> Result<Meeting, StorageError>;
    fn list_meetings(&self) -> Result<Vec<MeetingSummary>, StorageError>;
    fn update_meeting_ended(&self, id: &str, ended_at: &str, duration_ms: u64) -> Result<Meeting, StorageError>;
    fn update_meeting_language(&self, id: &str, language: &str) -> Result<Meeting, StorageError>;
    fn delete_meeting(&self, id: &str) -> Result<(), StorageError>;
    fn create_note(&self, draft: NoteDraft) -> Result<Note, StorageError>;
    fn list_notes_for_meeting(&self, meeting_id: &str) -> Result<Vec<Note>, StorageError>;
}

pub struct MeetingDraft {
    pub id: String,            // caller-generated UUIDv4 hex
    pub title: String,
    pub started_at: String,    // RFC3339
    pub language: String,      // BCP-47, e.g. "en", "es", "auto"
    pub dir_path: String,      // absolute path to meetings/<uuid>/
}

pub struct Meeting {
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub language: String,
    pub dir_path: String,
}

pub struct MeetingSummary {
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub duration_ms: u64,        // 0 if not yet stopped (for protocol non-Optional shape compat with slice 05)
}

pub struct NoteDraft {
    pub id: String,             // UUIDv4
    pub meeting_id: String,
    pub dialect: String,        // "notion-enhanced" | "basic"
    pub content_md: String,
    pub primary_path: String,   // absolute path to notes.md on disk
}

pub struct Note {
    pub id: String,
    pub meeting_id: String,
    pub dialect: String,
    pub content_md: String,
    pub primary_path: String,
    pub created_at: String,
}

pub enum StorageError {
    NotFound { id: String, kind: &'static str },     // kind = "meeting" | "note"
    AlreadyExists { id: String, kind: &'static str },
    SchemaMismatch { actual: u32, expected: u32 },
    Io { source: std::io::Error },           // #[from]
    Sql { message: String },                 // not rusqlite::Error — keeps panops-core rusqlite-free
}
// MUST NOT derive Serialize per AGENTS.md.
// `Sql { message }` (not `Sql { source: rusqlite::Error }`) so `panops-core`
// has no rusqlite dep. The adapter constructs via `StorageError::sql<E: Display>`
// helper. Trade-off: callers can't downcast to the original rusqlite::Error type;
// domain code never inspects, so acceptable.
```

The trait is sync (matches the existing `LlmProvider`, `AsrProvider`, `Diarizer`, `NotesExporter` ports). Async wrapping happens at the handler boundary via `tokio::task::spawn_blocking`.

### DB schema (single file, version 1)

DDL applied on first open by `RusqliteStorage::new(path)`:

```sql
PRAGMA user_version = 1;

CREATE TABLE IF NOT EXISTS meeting (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL,           -- RFC3339
    ended_at TEXT,
    duration_ms INTEGER,
    language TEXT NOT NULL DEFAULT 'auto',
    dir_path TEXT NOT NULL UNIQUE
);

CREATE INDEX IF NOT EXISTS idx_meeting_started_at ON meeting(started_at DESC);

CREATE TABLE IF NOT EXISTS note (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meeting(id) ON DELETE CASCADE,
    dialect TEXT NOT NULL,
    content_md TEXT NOT NULL,
    primary_path TEXT NOT NULL,
    created_at TEXT NOT NULL            -- RFC3339
);

CREATE INDEX IF NOT EXISTS idx_note_meeting_id ON note(meeting_id);
```

`PRAGMA user_version` is checked on `RusqliteStorage::new`; if it's `0` we treat the DB as freshly-created and run the DDL above; if it's `>= 1 && != EXPECTED` we return `SchemaMismatch`. (assistant default)

`PRAGMA foreign_keys = ON` set on every connection (rusqlite default is OFF). (assistant default)

### Filesystem layout

```
~/Library/Application Support/panops/
├── panops.db                            # registry (this slice's whole DB)
├── engine.sock                          # existing UDS (slice 05)
└── meetings/
    └── <uuid>/                          # one dir per meeting
        ├── notes.md                     # canonical note location for IPC-created meetings
        └── screenshots/                 # empty until Anchor B
```

`meetings/<uuid>/meeting.db` lands in Anchor B, not this slice.

### DI — `EngineServices` shape

`EngineServices` lives at `crates/panops-engine/src/server/mod.rs:71`. Today it has two fields: `llm: Arc<dyn LlmProvider>` (direct because `GenaiLlm::with_handle` is instant) and `heavy: Arc<OnceLock<Result<HeavyAdapters, String>>>` (deferred, holds `asr/diar/exporter` whose models load 20s+).

`Storage` is cheap to open (SQLite file open is microseconds, no model load). It joins the **direct** lane next to `llm`, NOT the OnceLock. `data_dir` is also direct.

```rust
pub struct EngineServices {
    pub llm: Arc<dyn LlmProvider + Send + Sync>,
    pub storage: Arc<dyn Storage>,                                          // NEW
    pub data_dir: PathBuf,                                                  // NEW
    pub(super) heavy: Arc<OnceLock<Result<HeavyAdapters, String>>>,
}

impl EngineServices {
    pub fn ready(
        llm: Arc<dyn LlmProvider + Send + Sync>,
        storage: Arc<dyn Storage>,                                          // NEW
        data_dir: PathBuf,                                                  // NEW
        asr: Arc<dyn AsrProvider + Send + Sync>,
        diar: Arc<dyn Diarizer + Send + Sync>,
        exporter: Arc<dyn NotesExporter + Send + Sync>,
    ) -> Self { /* ... */ }

    pub fn pending(
        llm: Arc<dyn LlmProvider + Send + Sync>,
        storage: Arc<dyn Storage>,                                          // NEW
        data_dir: PathBuf,                                                  // NEW
    ) -> (Self, Arc<OnceLock<Result<HeavyAdapters, String>>>) { /* ... */ }
}
```

`run_serve` constructs `RusqliteStorage::new(data_dir.join("panops.db"))?` **before** binding the socket. SQLite open is microseconds (no model load, no I/O budget concern), so the eager-after-bind pattern from #74 — which exists specifically to defer multi-second Whisper / Sherpa init past the 5s "socket appears" test budget — does NOT apply to storage. Opening before bind means a corrupt or wrong-schema-version DB fails with a clean exit code BEFORE we claim the socket, instead of leaving a dangling socket file behind a startup failure. (Spec amended 2026-05-09 from earlier "after bind" wording, ratified by maintainer; the code has always done this.)

Each integration test that uses `run_serve_in_process` injects its own `Arc<InMemoryStorage>` + `tempfile::TempDir` for `data_dir`.

### IPC surface

Wire types added to `crates/panops-protocol/src/methods.rs`. Keep `chrono`-free (slice 05 invariant; `started_at` stays a `String`).

```rust
// extension

pub struct MeetingConfig {
    pub title: Option<String>,           // defaults to "" if None
    pub language: Option<String>,        // defaults to "auto" if None
}

pub struct Meeting {                     // NEW in slice 06 (slice 05 only shipped MeetingSummary)
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub language: String,
    pub dir_path: String,
}

// MeetingSummary already exists at slice 05 (id, title, started_at, duration_ms: u64);
// keep its shape unchanged. duration_ms stays u64, defaulting to 0 for in-progress meetings.

#[derive(serde::Deserialize)]
pub struct NotesGenerateParams {
    pub audio: String,
    pub language: Option<String>,        // already exists at slice 05 (post-Copilot fix wave)
    pub dialect: Option<NotesDialect>,   // already exists at slice 05
    pub llm_provider: Option<String>,    // already exists at slice 05
    pub llm_model: Option<String>,       // already exists at slice 05
    pub no_diarize: Option<bool>,        // already exists at slice 05
    pub meeting_id: Option<String>,      // NEW — None = auto-create
}

pub struct NotesGenerateResult {
    pub primary_file: String,            // already exists
    pub assets: Vec<String>,             // already exists
    pub meeting_id: String,              // NEW — always set after slice 06
}
```

**No wire-breaking changes**, per the slice-05 forward-compat invariant ("new variants and new optional fields are non-breaking" — `docs/proto/ipc.md`). `Meeting` is newly introduced. `MeetingSummary` shape is unchanged. `NotesGenerateParams` adds optional `meeting_id`. `NotesGenerateResult` adds `meeting_id` (server emits; clients that ignore unknown fields keep working; the slice-05 design explicitly does NOT use `#[serde(deny_unknown_fields)]` on params or results).

### Concurrency & error model

`RusqliteStorage` holds `Arc<Mutex<rusqlite::Connection>>`. All trait methods take `&self` and are called from inside `tokio::task::spawn_blocking` at the handler layer. The mutex serializes writes; reads are not concurrent (single-user constraint, acceptable for v0.1).

All `StorageError` variants convert to `IpcError` at the protocol boundary via `From<StorageError> for IpcError` (gated behind the `domain-conversions` feature flag, matching slice-05 `AsrError`/`LlmError`/etc.):

| `StorageError` | `IpcError` | Wire message |
|---|---|---|
| `NotFound { id, kind }` | `InputNotFound` | `"<kind> not found"` (no id leak) |
| `AlreadyExists { id, kind }` | `InvalidInput` | `"<kind> already exists"` |
| `SchemaMismatch` | `Internal` | `"storage schema mismatch"` |
| `Io` | `Internal` | `"storage io error"` |
| `Sql` | `Internal` | `"storage error"` |

Per the slice-05 hardening pattern: full path / SQL / error chain goes to `tracing::error!`; the wire message stays opaque.

### `meeting.delete` ordering

1. Read meeting from DB to capture `dir_path`.
2. Delete meeting row (cascades to `note` rows via FK).
3. `std::fs::remove_dir_all(dir_path)`. On error: `tracing::warn!` with the path; return Ok (registry is the source of truth; orphan dir reaper is out of scope).

If step 2 fails: return error; nothing touched. If step 1 fails with `NotFound`: return `InputNotFound`.

### `notes.generate` extension

Pseudocode:

```rust
async fn notes_generate(svc: &EngineServices, params: NotesGenerateParams) -> Result<NotesGenerateResult, IpcError> {
    let audio_path = canonicalize_under_allowlist(&params.audio)?;  // existing slice-05 hardening
    let meeting_id = match params.meeting_id {
        Some(id) => {
            let _ = svc.storage.get_meeting(&id)?;                  // verify exists; enrich if needed
            id
        }
        None => {
            let id = Uuid::new_v4().simple().to_string();
            let dir = svc.data_dir.join("meetings").join(&id);
            std::fs::create_dir_all(dir.join("screenshots"))?;
            svc.storage.create_meeting(MeetingDraft {
                id: id.clone(),
                title: audio_path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
                started_at: now_rfc3339(),
                language: params.language.clone().unwrap_or_else(|| "auto".into()),
                dir_path: dir.to_string_lossy().into_owned(),
            })?;
            id
        }
    };
    // existing pipeline (asr → diar → llm → exporter), but exporter writes to:
    //   - meeting.dir_path  (when auto-created or existing meeting)
    //   - audio_path.parent().join(<stem>-notes)  (CLI-only legacy path; see §CLI behavior)
    // ... existing spawn_blocking + JobDone/JobError event flow ...
    let note = svc.storage.create_note(NoteDraft { id: Uuid::new_v4()..., meeting_id: meeting_id.clone(), ... })?;
    Ok(NotesGenerateResult { primary_file: ..., assets: ..., meeting_id })
}
```

### CLI behavior

| Surface | Before slice 06 | After slice 06 |
|---|---|---|
| `panops <wav>` (default mode) | Print JSON to stdout | **Unchanged.** No registry write. Default mode produces no notes artifact; a meeting-without-notes row would be misleading state. (Amended 2026-05-05 from the brainstorm; ratified by maintainer.) |
| `panops notes <wav>` | Write notes to `<audio_dir>/<stem>-notes/` | Same, plus register meeting (and a `note` row pointing at `notes.md`) in the registry. `dir_path = <audio_dir>/<stem>-notes/` (the legacy on-disk location), NOT `meetings/<uuid>/`. The registry just records the location; the IPC `notes.generate` path uses the canonical `meetings/<uuid>/` layout. Two flows, one registry. |
| `panops-engine serve` | Single mode | Add `--data-dir <path>` flag |

`--data-dir` defaults to `~/Library/Application Support/panops/` on macOS. Tests pass an explicit `tempfile::TempDir` path.

## Test surface

PR-gating tests:

**Conformance harness (panops-core)** — one suite, two adapters:
1. `storage_conformance` — runs against `InMemoryStorage` AND `RusqliteStorage` via a `#[test]` per adapter; covers create/get/list/update/delete for both `meeting` and `note`, plus `NotFound`, `AlreadyExists`, `SchemaMismatch`, `IoError`, and FK-cascade-on-meeting-delete.

**Integration (panops-engine/tests)** — one file per concern:
2. `ipc_meeting_list_real_storage` — supersedes slice-05's `ipc_meeting_list_returns_empty`. Asserts empty when the storage is empty, then creates rows via `Storage` directly and asserts they appear.
3. `ipc_meeting_start_creates_row_and_dir` — calls `ipc.meeting.start`, verifies row exists in DB, dir exists on disk with `screenshots/` subdir, `started_at` is server-set RFC3339.
4. `ipc_meeting_stop_writes_ended_at` — start then stop, verify `ended_at` and `duration_ms` populated.
5. `ipc_meeting_get_returns_full` — get the meeting created above.
6. `ipc_meeting_delete_removes_row_and_dir` — start → delete; verify no row, no dir, FK-cascaded notes also gone.
7. `ipc_meeting_set_language_persists` — set language, get back, verify.
8. `ipc_notes_generate_auto_creates_meeting` — call `notes.generate` with no `meeting_id`; verify result includes a new `meeting_id` and `meeting.list` returns one row.
9. `ipc_notes_generate_with_existing_meeting_id` — `meeting.start` then `notes.generate` with that id; verify the note row links to that meeting.
10. `ipc_notes_generate_with_unknown_meeting_id` — pass a bogus id; verify `InputNotFound` over the wire.
11. `ipc_persistence_survives_restart` — start meeting in process A (in-process via `run_serve_in_process` against a `tempfile::TempDir`), tear down, start a new in-process server pointing at the same dir, `meeting.list` returns the meeting from process A.

Existing slice-05 integration tests (`ipc_server_starts_and_binds`, `ipc_socket_perms_are_0600`, `ipc_stale_socket_is_cleaned`, `ipc_refuses_to_steal_live_socket`, `ipc_method_not_found_carries_jsonrpc_error`, `ipc_notes_generate_round_trip`, `ipc_job_error_carries_kind`) MUST continue to pass.

Total: 1 conformance suite (× 2 adapters) + 10 IPC integration tests + 7 retained slice-05 tests = **PR-gating count: 18 tests** (one of which has two adapter instances).

## Implementation order (sketch)

Canonical task list goes in the writing-plans output. This is illustrative.

1. Add `Storage` port + types + `StorageError` to `panops-core`. No impl yet. Compile & clippy clean.
2. Write `storage_conformance` harness. No adapter yet (compile only).
3. Implement `InMemoryStorage` fake (HashMap-backed). Wire into conformance harness; `cargo test` passes.
4. Add `rusqlite` dep + `RusqliteStorage::new` (open + DDL + version check). `storage_conformance` runs against it; `cargo test` passes.
5. Add `Storage` and `data_dir` to `EngineServices`. Construct `RusqliteStorage` in `run_serve`. Existing slice-05 tests still pass (with `Arc::new(InMemoryStorage::new())` injected for in-process tests).
6. Replace `meeting.list` stub with real impl. Update slice-05 test or add new one.
7. Add `meeting.start` handler + integration test.
8. Add `meeting.stop` handler + integration test.
9. Add `meeting.get` + `meeting.set_language` handlers + tests.
10. Add `meeting.delete` handler (with fs cleanup) + integration test.
11. Extend `notes.generate` with `meeting_id` (auto-create + existing + unknown-id paths) + 3 integration tests.
12. Add `--data-dir` CLI flag. Update CLI smoke tests.
13. Wire CLI default and `notes` modes to register the meeting (post-pipeline). Update `tests/cli_smoke.rs` (or equivalent).
14. Add `ipc_persistence_survives_restart` integration test.
15. Update `docs/proto/ipc.md`.
16. File deferred items as `type:debt` issues with `severity:` + `area:storage` (or `area:ipc`).

## Three-tier boundaries

Per AGENTS.md "every slice spec MUST define them".

### ✅ Always do

- Run `cargo fmt && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` before claiming any task done.
- Wrap every `RusqliteStorage` call site in `tokio::task::spawn_blocking` at the handler.
- Use `tempfile::TempDir` for every test that touches disk.
- Insert meeting + initial note rows in a single transaction when both happen in one handler call.
- Sanitize wire-side error messages: opaque "<kind> error" externally, full detail to `tracing::error!`.
- Commit per task in the slice plan (per L3 autonomy in AGENTS.md).
- File a follow-up `type:debt` GitHub issue for any "deferred" / "out of scope" item discovered during implementation.
- Run `storage_conformance` against both `InMemoryStorage` and `RusqliteStorage`.
- Drive `OnceLock` / `OnceCell` slots to a terminal state on every path including panic (per AGENTS.md `OnceLock` rule).

### ⚠️ Ask first

- Renaming a public protocol type (`Meeting`, `MeetingConfig`, `MeetingSummary`, `NotesGenerateResult`, `MeetingDraft`, `NoteDraft`).
- Adding `chrono` to `panops-protocol` (intentionally date-dep-free per `methods.rs:107` comment; `started_at` stays a `String` on the wire). `chrono` is already on `panops-core` / `panops-portable` / `panops-engine` — use it server-side for parsing/formatting RFC3339, just don't leak it into the protocol crate.
- Introducing a new `panops-portable` crate dep beyond `rusqlite` itself (e.g., `r2d2-rusqlite` for a connection pool).
- Choosing `rusqlite` features beyond `bundled` (e.g., `chrono`, `serde_json`, `blob`, `array`).
- Bundling a non-#17 / non-#75 issue into the slice (drift §3 NotionEnhanced default, drift §1 env-var cleanup, etc.).
- Changing the schema after the first commit lands (forces `user_version` bump + spec amendment + migration discussion).
- Removing or repurposing the existing CLI `panops notes` flow.
- Introducing WAL mode or per-task connections.
- Changing the file layout (`meetings/<uuid>/...`) after the first integration test lands.

### 🚫 Never do

- Add an env var for data dir, db path, or any user-facing config (per drift §1, AGENTS.md, north-star).
- Ship a public IPC method without an integration test.
- Auto-merge a PR (slice-05 lesson).
- Auto-file new architectural concerns as issues without surfacing to the maintainer first (slice-05 audit §5).
- Pre-trait a `Storage` variant — one trait, one real impl, one fake, period.
- Create the per-meeting `meeting.db` (defer to Anchor B).
- Create `segment / speaker / screenshot / job` tables (defer to Anchor B).
- Phone home or log to disk anything that contains user content beyond what's already persisted.
- Derive `serde::Serialize` on `StorageError` or any other domain error (per AGENTS.md).
- Open a PR autonomously. The maintainer opens PRs; commits within the slice plan don't need per-commit approval (L3) but PR open is L2.

## Decisions (locked)

- **D1**: Storage port = sync trait; async wrapping happens at handler via `spawn_blocking`. Reason: matches existing port shape (`AsrProvider`, `LlmProvider`, etc.) and keeps `panops-core` async-runtime-free.
- **D2**: Single registry DB at `panops.db`; per-meeting DB deferred to Anchor B. Reason: no per-meeting data exists yet; the split would buy nothing now.
- **D3**: `meeting.start` / `meeting.stop` ship as data-plane scaffolding. Reason: lets Mac shell (Anchor A) wire to them immediately; Anchor B layers on capture coupling.
- **D4**: `notes.generate` extends with optional `meeting_id`; auto-create when `None`. Reason: non-breaking IPC extension.
- **D5**: `RusqliteStorage` uses `Arc<Mutex<Connection>>`. Reason: single-user; pool overhead unjustified.
- **D6**: `rusqlite` with `bundled` feature, no system sqlite. Reason: predictable CI, no dynamic-lib pain (slice-04 #34 lesson).
- **D7**: `--data-dir` flag, no `PANOPS_DATA_DIR` env var. Reason: drift §1 + AGENTS.md.
- **D8**: PRAGMA `user_version` for schema versioning; mismatch = hard error. Reason: YAGNI for v0.1; migration framework is a v0.2 decision.
- **D9**: `meeting.delete` orders registry-then-fs; orphan dir is a logged warning, not a failure. Reason: registry is the source of truth.
- **D10**: `NotionEnhanced` remains the default `note.dialect`. Reason: changing the default is orthogonal to slice 06's storage focus; bundling expands scope. **Surfaced as Open question 1 for separate decision.**
- **D11**: New `Meeting` type uses `Option<u64>` for `duration_ms` and `Option<String>` for `ended_at`; existing `MeetingSummary` keeps `u64 duration_ms` unchanged (defaults to 0 for in-progress). Reason: in-progress meetings can't honestly report a duration; keeping `MeetingSummary` shape preserves slice-05 forward-compat.
- **D12**: All `StorageError` variants map to `IpcError` via the existing `domain-conversions` feature-flag pattern. `NotFound` → `InputNotFound`; everything else internal. Reason: matches slice-05 transport boundary.

## Open questions (out of slice 06; surface for separate decision)

1. **Drift §3 — `NotionEnhanced` as current default**. The May 2 alignment audit flagged that the genesis quote (`f0690f89:150`) framed Notion as "future phase" but code defaults to `NotionEnhanced`. Slice 06 preserves the default to avoid bundling unrelated change. Recommendation: maintainer decides before or after slice 06 whether to flip; if flipped, re-emit slice-04 goldens. One-line code change.
2. **Schema migration framework for v0.2**. PRAGMA `user_version` + hard-error works for v0.1. v0.2 needs `refinery` or `sqlx-migrate` or hand-rolled migrations. Decide when v0.2 begins.
3. **Concurrency at scale**. `Arc<Mutex<Connection>>` is single-user-fine; if real usage shows contention, evaluate WAL mode + per-task connections. File as debt only if it surfaces.
4. **Title source for auto-created meetings**. CLI uses audio file stem; Mac shell will let user enter one. The IPC `meeting.start(MeetingConfig)` accepts an optional title. Acceptable for v0.1; revisit with Mac shell UX.
5. **Cross-meeting search**. Not in slice 06. File as debt when needed.

## Done when

- All 18 PR-gating tests pass (`storage_conformance` × 2 adapters + 10 new IPC integration tests + 7 retained slice-05 tests; counts in §Test surface).
- `cargo fmt && cargo clippy --workspace --all-targets --locked -- -D warnings` is clean.
- Manual smoke succeeds: start `panops-engine serve --data-dir /tmp/panops-smoke`, run `panops --data-dir /tmp/panops-smoke notes tests/fixtures/audio/multi_speaker_60s.wav`, send `{"jsonrpc":"2.0","id":1,"method":"ipc.meeting.list","params":{}}` over the UDS — expect one row. Kill engine, restart with same `--data-dir`, repeat — still one row. Send `ipc.meeting.delete` with that id; `ipc.meeting.list` returns `[]`; `meetings/<id>/` directory gone.
- `docs/proto/ipc.md` updated.
- Deferred items filed as `type:debt` issues with `severity:` + `area:` labels and added to the project board.
- Slice-tracking issue closed; milestone `slice-06-storage` closed via `gh api -X PATCH`.
- Plan file moved to `docs/superpowers/plans/done/06-storage.md`.
- Slice-boundary alignment audit run after PR merge, written to `docs/superpowers/reviews/YYYY-MM-DD-slice-06-audit.md`.

## References

- Brainstorm: `docs/superpowers/specs/2026-05-05-slice-06-storage-brainstorm.md`
- Locked design (storage section): `docs/superpowers/specs/2026-04-30-panops-design.md:222-252`
- Slice 05 IPC spec (sibling, sets the protocol-crate pattern): `docs/superpowers/specs/2026-05-02-slice-05-ipc-design.md`
- May 2 alignment audit (recommendations #1, #2, #3, #5): `docs/superpowers/reviews/2026-05-02-alignment-audit.md`
- Tracking issues: #17 (https://github.com/vfmatzkin/panops/issues/17), #75 (https://github.com/vfmatzkin/panops/issues/75)
- AGENTS.md: workflow contract, three-tier boundaries rule, `OnceLock` rule, no-Serialize-on-domain-errors rule, no-env-vars rule
- North star: `docs/north-star.md` — v0.1 acceptance criterion #4 (notes persist across app restarts)
- rusqlite docs: https://docs.rs/rusqlite/0.31/
