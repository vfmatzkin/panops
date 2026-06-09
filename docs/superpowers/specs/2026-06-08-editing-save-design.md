# Editing & save — title/notes editing, autosave, drag-drop org, AI-tag suggestions — design

**Date:** 2026-06-08
**Status:** Approved for autonomous build (maintainer approved the design 2026-06-08, delegated solo build while away).
**Advances:** App usability — makes the app's content editable and reliably saved, surfaced from real usage feedback (title edits didn't persist, notes were render-only, saving was opaque, no drag-drop, tags weren't AI-assisted).

## Goal

Make the Mac app's content **editable and reliably saved**, with the save state always visible. Bridge the LLM's already-proposed tags into the org system, and add drag-and-drop organization. Decisions (maintainer-approved): **autosave + status indicator** (no save button); **raw-markdown notes editing with live preview**; **AI tags proposed at notes-generation** as accept/reject chips.

Single-user, local-first, no telemetry, no SaaS-isms. No backwards-compat shims (app + engine ship together; the DB may be reset during dev).

## Components

### 1. Title editing (autosave)
New IPC `meeting.rename { meeting_id, title }` → engine handler → storage `rename_meeting(meeting_id, title)` (updates the registry `meeting.title`). The existing inline title `TextField` in `MeetingDetailView` (currently `.onSubmit { /* local only */ }`) calls it on submit/blur and drives the save status.

### 2. Notes editing (raw markdown, autosave)
New IPC `notes.save { meeting_id, markdown }` → engine writes the meeting's `notes.md` and updates the `note` row (reuse the existing note storage). `NotesView` gains an **Edit** toggle: rendered view ↔ a markdown `TextEditor` bound to the `notes.md` source. On blur / toggle-out it autosaves and re-renders from the saved markdown (the existing markdown→sections fallback parses `notes.md`).

**Source-of-truth nuance:** after a manual edit, `notes.md` is authoritative and the rendered view re-parses it. The original `notes.json` IR stays as the LLM artifact (still read for #5's proposed tags). Regenerating notes re-derives both `notes.md` + `notes.json`.

### 3. Save status (fixes "saving is unknown")
A shared `SaveStatus` enum (`idle` / `saving` / `saved` / `failed(retry)`) shown near the title/toolbar, driven by every autosave op (title, notes, org). `Saved` fades after a beat; `failed` shows a **Retry**. No save button. **Failures are always surfaced — never a silent data loss.**

### 4. Drag-and-drop organization
SwiftUI `.draggable` on meeting rows + `.dropDestination` on the sidebar's Spaces / Projects / Tags rows → calls the **existing** `meeting.assign` / `tag.assign` IPC (no new backend) → refresh + save status. Context menus stay as the alternative.

### 5. AI-proposed tags
The notes IR (`notes.json`) frontmatter already carries LLM-proposed `tags` (`crates/panops-core/src/notes/ir.rs` → `StructuredNotes.tags`). Surface them as **pending suggestion chips** on the meeting, visually distinct from accepted/assigned tags. **Accept** → `tag.create` (idempotent on name) + `tag.assign` → becomes a real org tag; **dismiss** → hidden; **＋ add your own** → same create+assign path. The bridge is small — the model already produces the tags.

## IPC additions (`panops-protocol` + engine handlers)
- `meeting.rename { meeting_id: String, title: String } -> Meeting` (or unit) — wire type + handler + `Storage::rename_meeting`.
- `notes.save { meeting_id: String, markdown: String } -> ()` — handler writes `<meeting_dir>/notes.md` (via `PathValidator`) + updates the `note` row; wire type.
- Reuse: `meeting.assign`, `tag.create`/`tag.assign`/`tag.unassign`, `notes.json`/`notes.generate`.
- Domain error types stay non-`Serialize` (project rule). Conformance/fake updated for the new Storage method.

## App (`apps/Panops`)
- `IpcClient`: `renameMeeting`, `saveNotes` client methods (+ reuse assign/tag).
- `SaveStatus` model + a small status view; autosave wiring in the view model.
- `NotesView`: edit toggle + markdown `TextEditor` + re-render.
- Meeting-row `.draggable` + sidebar `.dropDestination`.
- AI-tag pending-chips view reading `notes.json` tags, with accept/dismiss/add.

## Build order (one slice, staged PRs — fleet-built, dual-model-reviewed, maintainer-merged)
1. **Backend (Rust):** `meeting.rename` + `notes.save` IPC + `Storage::rename_meeting` + note-update + wire types + conformance/fake + round-trip + socket tests. No UI yet.
2. **App edit + autosave + status (Swift):** save-status model/view; title autosave; notes edit toggle + markdown editor + autosave + re-render. Wires to stage 1.
3. **Drag-and-drop org (Swift):** draggable rows + droppable sidebar over the existing assign IPC.
4. **AI-tag suggestions (Swift):** pending chips from `notes.json` tags → accept (`tag.create`+`tag.assign`) / dismiss / add.

Each stage = isolated-surface PR. (The screenshots-from-video slice is separate, after.)

## Three-tier boundaries
- ✅ Always: `cargo fmt`/`clippy` + `swift build`/`swift test` before pushing; commit per stage-step; autosave MUST surface failures (no silent data loss); reuse existing assign/tag IPC for drag-drop; open issues for deferred items.
- ⚠️ Ask-first: changing the `notes.md` on-disk format or the `note`/`meeting` table schema beyond the additive title update; reconciling `notes.md` ↔ `notes.json` divergence beyond "md wins after manual edit"; autosave debounce semantics if it gets chatty.
- 🚫 Never: SaaS-isms; telemetry; losing a user edit silently; a new port/trait without a real impl + fake; opening/merging a PR autonomously; user-config env vars; back-compat shims.

## Verification
- Rust: `cargo build/test/clippy --workspace --locked` incl. socket tests; `meeting.rename`/`notes.save` wire round-trips; a socket test that renames a meeting + reads it back, and that `notes.save` writes `notes.md` + the note row.
- Swift: `swift build` + `swift test` (note: CI's macos-latest is Swift 6.1, stricter than local — watch `@MainActor`/Sendable on any new async UI code; `@preconcurrency import` for non-Sendable Apple frameworks).
- Manual smoke (signed bundle): edit a title → reload → persists; edit notes → blur → "Saved" → reload → persists + re-renders; drag a meeting onto a Space/Tag → assigned; accept an AI-proposed tag → becomes a real tag chip.

## Out of scope (deferred → debt)
- WYSIWYG/rich notes editing (raw markdown first).
- Editing the structured IR fields directly.
- Drag-reordering Spaces/Projects (assignment DnD only).
- `notes.md` ↔ `notes.json` conflict reconciliation beyond "md wins after a manual edit."
- Undo/redo for edits.
