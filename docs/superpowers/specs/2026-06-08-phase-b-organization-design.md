# Slice: Phase B — organization (Spaces / Projects / Tags) — design

**Date:** 2026-06-08
**Status:** Approved for autonomous build (maintainer delegated solo design+build while away, 2026-06-08).
**Advances:** the UX plan's Phase B (organization), on top of the shipped Phase A workspace.

## Goal

Let the user organize meetings beyond a flat date-grouped list: group them into **Spaces** (e.g. Work / Study / Personal), optionally into **Projects** within a space, and label them with **Tags**. Unassigned meetings live in an implicit **Inbox**. Single-user, on-device, no accounts/members/quotas (open-source local-first).

## Product model

- **Space** — a top-level user-created grouping (id, name, position). No hardcoded names.
- **Project** — a collection inside exactly one Space (id, space_id, name, position).
- **Tag** — a free label (id, name unique); many-to-many with meetings via `meeting_tag`.
- **Meeting** gains `space_id` (nullable) + `project_id` (nullable). Rules: a meeting has 0–1 space and 0–1 project; assigning a project sets the meeting's space to that project's space (project ⊂ space); `space_id IS NULL` ⇒ **Inbox** (no Inbox row exists — it's the null bucket). Deleting a space/project nulls the affected meetings' refs (meetings are never deleted by org changes).

## Architecture

Registry-only change — everything lives in `panops.db` (the cross-meeting registry), not per-meeting dbs. Extends the existing `Storage` port + its SQLite adapter + conformance/fake (no new port — these are storage concerns). IPC adds `space.*` / `project.* `/ `tag.*` / `meeting.assign` / `meeting.tag`. The app's `AppViewModel` + sidebar consume them. Domain types in `panops-core` stay platform-agnostic; wire types in `panops-protocol`.

## Storage migration

`rusqlite_storage.rs` is versioned via `PRAGMA user_version` / `EXPECTED_SCHEMA_VERSION`. Bump it and add a migration step:
```sql
CREATE TABLE space   (id TEXT PRIMARY KEY, name TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
CREATE TABLE project (id TEXT PRIMARY KEY, space_id TEXT NOT NULL REFERENCES space(id) ON DELETE CASCADE, name TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
CREATE TABLE tag     (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
CREATE TABLE meeting_tag (meeting_id TEXT NOT NULL REFERENCES meeting(id) ON DELETE CASCADE, tag_id TEXT NOT NULL REFERENCES tag(id) ON DELETE CASCADE, PRIMARY KEY (meeting_id, tag_id));
ALTER TABLE meeting ADD COLUMN space_id   TEXT REFERENCES space(id)   ON DELETE SET NULL;
ALTER TABLE meeting ADD COLUMN project_id TEXT REFERENCES project(id) ON DELETE SET NULL;
```
Migration must be forward-only + idempotent on the existing single-version registry; preserve existing meeting/note rows. Test: an old-version db migrates cleanly + existing data survives.

## Domain + Storage port (panops-core)

New domain types: `Space { id, name, position }`, `Project { id, space_id, name, position }`, `Tag { id, name }`. `MeetingSummary` gains `space_id: Option<String>`, `project_id: Option<String>`, `tags: Vec<String>` (tag names or ids — ids, with names resolvable via list_tags).

New `Storage` port methods (each added to the trait + the SQLite adapter + the conformance suite + the fake):
- spaces: `create_space(name) -> Space`, `list_spaces() -> Vec<Space>`, `rename_space(id, name)`, `delete_space(id)`.
- projects: `create_project(space_id, name) -> Project`, `list_projects(space_id) -> Vec<Project>` (and/or all), `rename_project`, `delete_project`.
- tags: `create_tag(name) -> Tag` (idempotent on name), `list_tags() -> Vec<Tag>`, `delete_tag(id)`, `tag_meeting(meeting_id, tag_id)`, `untag_meeting(meeting_id, tag_id)`, `list_tags_for_meeting(meeting_id) -> Vec<Tag>`.
- assign: `assign_meeting(meeting_id, space_id: Option, project_id: Option)` (project sets space).
- `list_meetings` gains optional filters (space_id / project_id / tag_id / unsorted) for the sidebar.

## IPC (panops-protocol + engine handlers)

`ipc.space.create|list|rename|delete`, `ipc.project.create|list|rename|delete`, `ipc.tag.create|list|delete|assign|unassign` (assign/unassign = tag/untag a meeting), `ipc.meeting.assign { meeting_id, space_id?, project_id? }`. `ipc.meeting.list` gains optional filter params. Wire types mirror the domain; domain↔wire conversions; conformance updated.

## App UI (apps/Panops)

Sidebar (`MeetingListView` / `ContentView` / `AppViewModel`):
- **Smart Views** section: Inbox (unsorted), All, Needs Notes, This Week — client-side filters over the meeting list.
- **Spaces** section: collapsible Spaces → Projects; selecting one filters the meeting list. "+" to create a space/project; rename/delete via context menu.
- **Tags** section: list tags; selecting filters; meetings show their tag chips.
- **Assign**: meeting context-menu → Move to Space/Project, Add/Remove Tag. (Drag-and-drop optional, later.)
- Reuse the date-grouped rich rows within the selected view.

## Build order (thin vertical slices → separate PRs)

- **PR B1 — storage foundation (Rust):** schema migration + bump version + domain types (Space/Project/Tag) + the Storage port methods + SQLite adapter + conformance + fake + MeetingSummary fields. The data layer, fully tested (incl. migration test). No IPC/UI yet.
- **PR B2 — IPC (Rust):** the `space.*`/`project.*`/`tag.*`/`meeting.assign`/`meeting.tag` methods + handlers + meeting.list filters + wire types. Stacked on B1.
- **PR B3 — sidebar UI (Swift):** Smart Views + Spaces→Projects + Tags + assign context-menus, wired to the new IPC. Stacked on B2. (May split B3a Spaces/Projects, B3b Tags + Smart Views if large.)

Each PR = isolated-surface, fleet-built, dual-model-reviewed, maintainer-merged.

## Three-tier boundaries

- ✅ Always: `cargo fmt`/`clippy` + `swift build`/`swift test` before pushing; commit per unit; migration must preserve existing data (test it); open issues for deferred items.
- ⚠️ Ask-first: any change to existing `meeting`/`note` columns beyond the additive `space_id`/`project_id`; any per-meeting-db change (this slice is registry-only); drag-and-drop reordering semantics.
- 🚫 Never: SaaS-isms (accounts, members, sharing, quotas, "X of N GB"); a destructive migration (must be additive + preserve data); telemetry; cloud sync; user-config env vars.

## Verification

Per PR: `cargo build/test/clippy --workspace --locked` (incl. socket tests + a migration test that an old-version registry upgrades + keeps its meetings); `swift build`+`swift test` (CLT framework paths). A real end-to-end (create space → assign meeting → filter) is exercised via the IPC integration tests + the app's fakes.

## Out of scope (deferred → debt)

- Drag-and-drop assignment (context-menu first).
- Calendar/Events, Participants/People, Sharing (north-star: future, opt-in).
- Smart Views beyond the four listed.
- Tag colors / space icons (names only first).
- Cross-meeting search beyond title/date/language.
