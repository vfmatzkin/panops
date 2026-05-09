# Slice 06 — SQLite storage + meeting lifecycle: brainstorm

**Status:** brainstorm artifact. Source for the locked spec at `2026-05-05-slice-06-storage-design.md`. Captures the 2026-05-05 interactive brainstorm session that walked the maintainer through scope, scaffolding semantics, and DB layout. Decisions made here are reflected in the locked spec.

## 0. Pickup state

Slice 05 (IPC) shipped clean on May 2 (`d22c0ab`). The three intervening days were cleanup: #74 (real adapters in `serve`) closed via PR #93, Claude GitHub Actions added (PR #92), Copilot review instructions added (PR #94), AGENTS.md amended with OnceLock terminal-state + domain-error no-Serialize rules (PR #104), dependabot bumps merged. **No active slice plan exists; the project board has 33 Todo items at brainstorm time.**

## 1. Trajectory choice — which slice next?

Per the trajectory in `AGENTS.md` (with #74 done), four options were on the table:

| Option | Argument | Argument against |
|---|---|---|
| **#17 SQLite persistence** | Smallest well-scoped slice; no new language surface; unblocks Anchor A AND v0.1 acceptance #4 | Doesn't visibly change product yet (no UI, no calibration win) |
| **Real-meeting calibration** | Hits v0.1 acceptance #5 (the "would I actually use this" gate) | Requires real meeting recording; doesn't unblock Anchor A code-wise |
| **Anchor A — Mac shell + ASR sidecar** | First time the product is usable in app form | Largest scope; lands Swift code against stub data plane and would need rework |
| **Audit drift cleanup first** | Knocks out 5 unresolved drift items before any new code | Small, doesn't justify a slice; blocks v0.1 progress |

**Maintainer chose**: #17 SQLite persistence (recommended).

## 2. Scope size — thin / medium / fat

Three thicknesses considered for #17:

- **Thin (walking-skeleton)** — Storage port + registry adapter only. `meeting.list` returns rows from a registry that only `meeting.start/stop` would populate, but those don't exist yet, so the registry stays empty. `notes.generate` continues writing markdown next to input audio with no DB record. Pure plumbing slice; doesn't actually hit acceptance #4 yet.
- **Medium (recommended at brainstorm time)** — Storage port + rusqlite registry + per-meeting DB for `note` only. Wire `notes.generate` to (a) write into canonical `meetings/<uuid>/`, (b) insert into registry, (c) return meeting id. `meeting.list` reads real rows.
- **Fat (#17 + #75)** — Above plus the new `meeting.start / stop / get / delete / set_language` IPC methods (currently filed as #75). Two slice-worth of work bundled.

**Maintainer chose**: Fat (#17 + #75). Reason: lifecycle methods are blocked on #17 and naturally land together; one slice ships a complete data plane the Mac shell can land against.

## 3. `meeting.start` / `meeting.stop` semantics — what do they mean before live capture?

Per design spec, `meeting.start(config)` opens the meeting dir + initializes `audio.m4a / video.mp4 / meeting.db / screenshots/`. Without ScreenCaptureKit (Anchor B), these methods don't have a "real" job to do. Three interpretations considered:

- **Data-plane scaffolding (recommended)** — `start` creates the row + dir + `screenshots/`, sets `started_at`. No capture coupling. `stop` writes `ended_at` and `duration_ms`. Anchor A (Mac shell) calls them when the user clicks record/stop; Anchor B layers on actual capture.
- **Defer entirely to Anchor B** — slice 06 ships only `meeting.list / get / delete / set_language`. Smaller scope; #75 stays partially open; Mac shell has incomplete lifecycle to call.
- **Import-existing-audio semantics** — re-shape `meeting.start(audio: String, language: String) -> meeting_id`. CLI uses this. Diverges from spec's `MeetingConfig` shape — spec amendment required.

**Maintainer chose**: Data-plane scaffolding. The locked spec captures this as decision D3.

## 4. DB layout — per-spec split now or single-DB now?

Design spec calls for two SQLite files: registry at `panops.db` (just `meeting_id → path` + global settings) and per-meeting at `meetings/<uuid>/meeting.db` (full `meeting + segment + speaker + screenshot + note + job`).

Three layouts considered:

| Layout | Pros | Cons |
|---|---|---|
| **Single-DB now (recommended)** | Simplest; `meeting.list` is one SELECT; no premature complexity for empty tables | Spec amendment; per-meeting DB lands when Anchor B needs it |
| **Per-spec split now** | Stays faithful to locked design; `meeting.delete` is just `rm -rf` + delete row | Two adapters or one with two connections; `meeting.list` = N+1 opens for nothing; `segment/speaker/screenshot/job` tables empty until Anchor B |
| **Hybrid (denormalised)** | Fast `meeting.list`; forward-compat with Anchor B | Two sources of truth for meeting metadata; write-time sync burden |

**Maintainer chose**: Single-DB now, per-meeting split deferred to Anchor B. Recorded as **deliberate spec amendment**, not silent drift.

## 5. Sub-decisions taken with assistant defaults (subject to maintainer veto via spec review)

These weren't asked individually because they're either AGENTS.md-mandated or low-risk:

- `Storage` trait is sync; async wrapping at handler via `spawn_blocking` — matches existing port pattern.
- `RusqliteStorage` uses `Arc<Mutex<Connection>>` — single-user; pool unjustified.
- `rusqlite` with `bundled` feature — predictable CI, no system-sqlite link pain.
- PRAGMA `user_version` for schema versioning; mismatch = hard error — YAGNI for v0.1.
- `--data-dir` CLI flag, no `PANOPS_DATA_DIR` env var — drift §1 + AGENTS.md.
- UUIDv4 simple form for meeting/note ids — `uuid` already a workspace dep.
- `meeting.delete` orders registry-row-then-fs; orphan dir = logged warning, not failure — registry is source of truth.
- `notes.generate` non-breaking extension with optional `meeting_id` — keeps existing IPC clients working.
- `Storage` port lives in `panops-core`; `RusqliteStorage` in `panops-portable`; `InMemoryStorage` fake in `panops-core::conformance::fakes` — matches existing port locations.
- `EngineServices` gains `Arc<dyn Storage> + data_dir: PathBuf` in the **direct** lane (next to `llm`), not in the `heavy` `OnceLock` — SQLite open is microseconds, no model load.
- New `Meeting` type uses `Option<u64> duration_ms` and `Option<String> ended_at`; existing `MeetingSummary` shape preserved.

## 6. Open questions surfaced (NOT for this slice — surfaced for separate decision)

1. **Drift §3 — `NotionEnhanced` as current default**. Slice 06 touches `note.dialect`. The May 2 alignment audit flagged that the genesis quote (`f0690f89:150`) framed Notion as "future phase" but code defaults to `NotionEnhanced`. Slice 06 preserves the default to avoid bundling unrelated change. **Maintainer decides separately.**
2. **Schema migration framework for v0.2**.
3. **Concurrency at scale** (`Arc<Mutex<Connection>>` is single-user-fine; revisit if real usage shows contention).
4. **Title source for auto-created meetings** (today: audio file stem; Mac shell will let user enter one).
5. **Cross-meeting search** — not in slice 06.

## 7. What this artifact is NOT

- Not the locked spec — that's `2026-05-05-slice-06-storage-design.md`.
- Not the executable plan — that's `docs/superpowers/plans/06-storage.md` (written by `superpowers:writing-plans` post-spec-approval).
- Not authoritative on architecture — the locked spec is. If they disagree, the spec wins.

## 8. References

- Plan file (in-flight brainstorm scratchpad, identical content): `~/.claude/plans/let-s-continue-with-the-steady-deer.md`
- Locked design (storage section): `docs/superpowers/specs/2026-04-30-panops-design.md:222-252`
- May 2 alignment audit: `docs/superpowers/reviews/2026-05-02-alignment-audit.md`
- Tracking: #17 (SQLite persistence), #75 (meeting.* IPC methods)
