# AGENTS.md — panops

Open-source local-first macOS recorder with screenshot-anchored meeting notes. Hexagonal Rust core + SwiftUI Mac shell + Swift sidecars (WhisperKit/FluidAudio for ASR, Apple FoundationModels for LLM).

This file IS the workflow contract. Every rule here is enforced. If a rule is unclear or you (the agent) disagree with it, raise it with the maintainer before acting — don't reinterpret silently.

## Sources of truth

| Surface | What it owns | Where |
|---|---|---|
| **GitHub Project board** | Open work, status, severity, slice ownership. **Lead with this.** | https://github.com/users/vfmatzkin/projects/1 (linked at https://github.com/vfmatzkin/panops/projects) |
| **Design spec** | Architecture (locked). Don't restructure ports/adapters without revising. | `docs/superpowers/specs/2026-04-30-panops-design.md` |
| **Slice spec** | The active slice's design (locked once approved). | `docs/superpowers/specs/<latest>-slice-NN-*.md` |
| **Slice plan** (private) | Step-by-step checklist for the active slice. Checkbox state IS progress. | `docs/superpowers/plans/<latest>.md` (not in `done/`) |
| **Session log** (private) | Journal of what happened in each work block. | `docs/superpowers/sessions/NNN-YYYY-MM-DD-<slug>.md` |
| **Market scan** | Verified library + pattern picks (April 2026). | `docs/research/2026-04-30-market-scan.md` |

When the project board and a markdown artifact disagree, the project board wins.

## Repo conventions

- Rust workspace, `resolver = "2"`. Crates: `panops-core` (domain + ports), `panops-portable`, `panops-mac` (`#[cfg(target_os="macos")]`), `panops-engine` (binary), `panops-protocol` (IPC types).
- Swift (slice 06+): SwiftUI app under `apps/Panops/`, sidecars `apps/panops-asr-mac/` and `apps/panops-llm-mac/`.
- Storage: SQLite per meeting at `~/Library/Application Support/panops/meetings/<uuid>/meeting.db`. Cross-meeting registry at `~/Library/Application Support/panops/panops.db`.
- IPC: Unix socket at `~/Library/Application Support/panops/engine.sock`. JSON-RPC 2.0 control + WebSocket events.
- Notes output: markdown + YAML frontmatter, `NotionEnhanced` dialect by default, `Basic` via toggle.
- License: MIT.

## Commands

```bash
cargo build --workspace
cargo test --workspace --locked
cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings
gh pr checks <num>            # CI status
gh issue list --label type:debt --state open --limit 50
gh project item-list 1 --owner vfmatzkin --format json
```

Mac app build commands land in slice 06.

## Execution discipline (MUST follow)

- **MUST** ship thin vertical slices end-to-end. Walking skeleton first.
- **MUST** ship the headless CLI before any Swift. Live capture last.
- **MUST** introduce one trait at a time + one real impl + one fake. **NEVER** pre-trait for hypothetical future adapters.
- **MUST** keep test fixtures in `tests/fixtures/` as adapters that satisfy the same conformance suite as live impls.
- **MUST** write a conformance fn per port. Every adapter passes the same harness.
- **MUST** keep one slice = one PR = the maintainer's review gate. **NEVER** start slice N+1 until slice N's PR is merged.
- **MUST** scope plans per-slice. **NEVER** write whole-project step lists.

## Slice sequence

1. Skeleton — Cargo workspace, fixtures (audio, video, screenshots).
2. Headless CLI — `AsrProvider` trait + `whisper-rs` adapter, JSON segments to stdout.
3. Post-pass + diarization — pyannote via sherpa-rs adapter, speaker labels merged.
4. Notes generation — `LlmProvider` port + `NotesGenerator` pipeline + `MarkdownExporter`.
5. IPC — JSON-RPC + WebSocket over Unix socket, exercised by Rust test client.
6. SwiftUI shell + ASR sidecar — WhisperKit + FluidAudio sidecar.
7. Live ScreenCaptureKit capture — risk-last, real audio + screen + screenshots.

Slices 1–3 shipped. Active slice: see project board milestones (`slice-04-notes-generation` etc.).

## Session rituals

### Pickup ritual (run at the START of every session)

1. Run `git status` and `git log -10 --oneline`.
2. Open the project board. Find the active slice milestone (issues with `Status: In Progress`, or the slice the latest session log is working on).
3. Read the most recent file in `docs/superpowers/sessions/` for context.
4. Read the active plan in `docs/superpowers/plans/` (latest file not in `done/`).
5. Identify next action: top-severity open issue in the active milestone with `Status: Todo` and no blocker, OR next unchecked step in the active plan.
6. State explicitly: `"Slice 0N is X/Y done. Last session: <slug>. Next: <issue #N or step>. Blockers: <list or 'none'>."`
7. **MUST** wait for the maintainer's "go" before resuming code changes.

### Handoff ritual (run when STOPPING for the session)

1. Update project-board state: move touched issues to `In progress` / `Done`. Open issues for any new debt (per Debt rule below).
2. Tick checkboxes in the active plan for completed steps.
3. Append new decisions to the plan's `Decisions made this slice` section.
4. Append new blockers to the plan AND to the matching project issue.
5. Write `docs/superpowers/sessions/NNN-YYYY-MM-DD-<slug>.md` (increment counter). Format: see `docs/superpowers/sessions/README.md`. Terse.
6. **NEVER** auto-commit. Leave changes staged or note them in the session log.
7. End message: `"Stopping at <step or issue #>. Next pickup: project board, then sessions/<latest>.md and plans/<active>.md."`

### When a slice ships (after the maintainer merges its PR)

1. Move `docs/superpowers/plans/<slice>.md` into `docs/superpowers/plans/done/`.
2. Close the slice's milestone on the project board (`gh api -X PATCH repos/vfmatzkin/panops/milestones/<n> -f state=closed`).
3. Brainstorm the next slice via the `superpowers:brainstorming` skill, then `superpowers:writing-plans`. **NEVER** queue multiple slice plans ahead.
4. Open a slice tracking issue for the new milestone (label `type:feature`, milestone = new slice's). Body links to spec + plan + key tasks.

### PR merge rule (MUST follow)

Before merging ANY PR — even one-line trivial changes:

1. **Wait for CI to finish** (`gh pr checks <num>` until all required checks are `pass`).
2. **Wait for Copilot's review** (`gh pr view <num> --json reviews` shows `copilot-pull-request-reviewer: COMMENTED` or similar). Copilot is automatic but isn't always immediate; give it a minute after the PR opens.
3. **Read every inline comment** (`gh api repos/vfmatzkin/panops/pulls/<num>/comments`). For each: either fix it and push, or post a reply explaining why you're declining (technical reasoning, not "it's fine").
4. **Resolve every thread** (`gh api graphql` with `resolveReviewThread`) once handled.
5. Only then `gh pr merge <num> --rebase --delete-branch`.

**NEVER** merge before Copilot has reviewed and threads are resolved. The bot catches real issues — the PRs in this repo's history (#11, #13, #28) all had legitimate Copilot findings that would have shipped without the wait.

## Debt rule (MUST follow)

Anytime a spec, plan, or session log says **"deferred"** / **"follow-up"** / **"out of scope"** / **"later slice"** / **"defer to"**, open a GitHub issue. Don't let debt live only in markdown.

**How to file:**

1. Use the tech-debt template: https://github.com/vfmatzkin/panops/issues/new?template=tech-debt.yml — or `gh issue create` with the body fields below.
2. Apply labels: `type:debt` + one `area:*` + one `severity:*`.
3. Add to the project board: `gh project item-add 1 --owner vfmatzkin --url <issue-url>`. Populate `Severity`, `Area`, `Slice introduced` fields.
4. Set milestone only if a specific slice will own the fix. Otherwise leave milestone-less (backlog).
5. Add `release:v0.1` label if it must land before the v0.1 release.

**Label taxonomy (canonical):**

- `type:` — `bug`, `feature`, `debt`, `docs`. One per issue.
- `area:` — `asr`, `diar`, `notes`, `llm`, `ci`, `storage`, `ipc`, `mac-shell`. One per issue.
- `severity:` — `critical`, `high`, `medium`, `low`. Only on `type:debt` and `type:bug`.
- `release:v0.1` — must land in v0.1. Label (not milestone) so it composes with slice ownership.

Issue templates live at `.github/ISSUE_TEMPLATE/`.

## Don'ts (NEVER do these)

- **NEVER** propose live-capture work or native API bindings before slice 6/7.
- **NEVER** add features beyond the active slice's plan.
- **NEVER** pre-trait. One trait when needed, one real impl, one fake.
- **NEVER** auto-commit or push. The maintainer commits when they want.
- **NEVER** give time estimates (no "ships in weeks", "X-day project", schedule framing). Compare options on capability/risk/cleanliness.
- **NEVER** phone home. Zero telemetry, ever, even opt-in.
- **NEVER** delete or rewrite files in `docs/superpowers/{specs,plans,sessions}/` or `docs/research/` without explicit instruction. They're history.
- **NEVER** leave a "deferred" item only in markdown — file it as an issue (see Debt rule).
- **NEVER** mark an issue as "wontfix" via label. Close as "not planned" with a comment instead.

## When in doubt

Re-read the design spec. Architecture is locked. Slice plans iterate; the design doesn't.

## Local additions

Personal notes that shouldn't ship publicly go in `AGENTS.local.md` (gitignored). Keep public-facing rules in this file.
