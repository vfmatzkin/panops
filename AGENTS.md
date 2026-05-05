# AGENTS.md — panops

Open-source local-first macOS recorder with screenshot-anchored meeting notes. Hexagonal Rust core + SwiftUI Mac shell + Swift sidecars (WhisperKit/FluidAudio for ASR, Apple FoundationModels for LLM).

This file IS the workflow contract. Every rule here is enforced. The methodology assumes **solo development with autonomous agents** — the human is *framework designer, not active monitor*. Per-decision gating is the failure mode; constraints up front + scheduled drift detection is the working mode. If a rule is unclear or you (the agent) disagree with it, raise it with the maintainer before acting — don't reinterpret silently.

For project goal + v0.1 acceptance criteria, see `docs/north-star.md`. For methodology rationale, see `docs/superpowers/reviews/2026-05-02-solo-agent-methodology.md`.

## Sources of truth

| Surface | What it owns | Where |
|---|---|---|
| **North Star** | Project goal + v0.1 acceptance criteria. The thing every slice must serve. Amend only at major-milestone boundaries. | `docs/north-star.md` |
| **GitHub Project board** | Open work, status, severity, slice ownership. **Lead with this for in-flight state.** | https://github.com/users/vfmatzkin/projects/1 (linked at https://github.com/vfmatzkin/panops/projects) |
| **Design spec** | Architecture (locked). Don't restructure ports/adapters without revising. | `docs/superpowers/specs/2026-04-30-panops-design.md` |
| **Slice spec** | The active slice's design (locked once approved). | `docs/superpowers/specs/<latest>-slice-NN-*.md` |
| **Slice plan** (private) | Step-by-step checklist for the active slice. Checkbox state IS progress. | `docs/superpowers/plans/<latest>.md` (not in `done/`) |
| **Session log** (private) | Journal of what happened in each work block. | `docs/superpowers/sessions/NNN-YYYY-MM-DD-<slug>.md` |
| **Alignment audit** (private) | Drift findings from the audit ritual. Latest one is the canonical drift state. | `docs/superpowers/reviews/YYYY-MM-DD-*-audit.md` |
| **Market scan** | Verified library + pattern picks (April 2026). | `docs/research/2026-04-30-market-scan.md` |

Priority when sources disagree: **north-star > active slice spec > design spec > slice plan**. North star is the goal; everything below is a translation. Project board wins for in-flight state (who's blocked, what's open).

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
- **MUST** drive `OnceLock` / `OnceCell` (or any once-init slot a handler can observe) to a terminal state on every path — success, error, or panic. Wrap initializing closures in `std::panic::catch_unwind` and convert panic payloads to `Err` so the slot never stays permanently `None`. Precedent: commit `02559a3` (heavy-init panic recovery).
- **MUST NOT** derive `serde::Serialize` on domain error types (`AsrError`, `DiarError`, `LlmError`, `NotesError`, …). IPC transport types in `crates/panops-protocol` convert from them at the boundary so domain errors stay platform-agnostic and free to evolve.

## Methodology — solo dev with agents

Per-decision gating is the failure mode; constraints up front + scheduled drift detection is the working mode. Built on Anthropic's *Effective harnesses for long-running agents* (humans-as-framework-designers), Harper Reed's spec → plan → execute pattern, Addy Osmani's three-tier boundaries, and Swarmia's autonomy levels.

### Autonomy levels

- **L2 — collaborative**: brainstorming, slice spec writing, architectural calls, library choice, PR review. Agent proposes; maintainer picks. **Never let an agent lock architecture autonomously.**
- **L3 — task agent**: slice plan execution. Agent runs the plan to PR; tests + diff are the gate; maintainer reviews PR, not steps. **Default for implementation work inside an approved slice.**
- **L4 — autonomous teammate**: only for genuinely recurring work (Dependabot, lint sweeps). Currently unused; experiment cautiously before adopting.

Architectural decisions = L2. Slice implementation = L3. Maintenance = L4. **NEVER** mix levels mid-slice without explicit agreement.

### Three-tier boundaries (every slice spec MUST define them)

Each slice spec includes a section listing, with verbs:

- ✅ **Always do** — actions agents take without asking (e.g., "run `cargo fmt`", "open issues for deferred items", "commit per task in the slice plan").
- ⚠️ **Ask first** — actions warranting a pause + brief confirmation (e.g., "rename a public type", "drop a CI check", "change a test threshold").
- 🚫 **Never do** — hard stops (e.g., "introduce a trait without a real impl + fake", "phone home", "open or merge a PR autonomously").

If a category can't be written upfront, it can't be decided upfront — escalate before the brainstorm finishes. Per Addy Osmani: *clearer constraints grant more independence*.

### Alignment-audit ritual

Drift detection runs **continuously, with automatic triggers** — the harness fires it; the maintainer doesn't have to remember. Three tiers:

#### Lightweight audit (frequent, context-rot driven)

Triggered when context is at risk:
- At SessionStart when the active session log is >24h old or the conversation has >100 turns since the last audit.
- After any `/compact` or auto-compaction event.
- Before opening any PR.
- After dispatching >5 subagents in one session (fan-out can drift on its own).
- Whenever the maintainer types "audit", "are we on track", or invokes `/ultrathink` review.

Procedure (≤5 min):
1. Re-read `docs/north-star.md`.
2. Re-read the active slice spec's three-tier boundaries.
3. Run `mcp__claude-review__research_project` with: *"Has the active slice's shipped state diverged from `docs/north-star.md` and the slice spec? Cite drifts with file:line."*
4. If drifts surface, post a one-line warning to the maintainer + file follow-up debt issues.

This is the context-rot mitigation. Run it without asking.

#### Slice-boundary audit (heavy, automatic post-merge)

Triggered when a slice's PR merges. Procedure (MCP-first per Tool routing):
1. `mcp__claude-review__audit_pr` against the merged commit range with `focus: all` — risk + style + tests passes.
2. `mcp__claude-review__inspect_transcript` against the slice's main session `.jsonl`. Extract user words vs shipped state.
3. `mcp__claude-review__research_project` for any topic the audit_pr surfaces but doesn't fully ground.
4. **Only if MCP can't reach the depth needed**: dispatch ONE `Task` subagent for a specific cross-cutting synthesis — never a parallel fan-out, never as the default.
5. Write findings to `docs/superpowers/reviews/YYYY-MM-DD-slice-NN-audit.md`. List drifts. File debt for non-trivial.
6. If a drift conflicts with `docs/north-star.md`, treat as **blocking** — fix or amend the north-star before the next slice's brainstorm.

#### Maintainer-triggered audit (on demand)

When the maintainer asks for a deep audit ("review alignment", `/ultrathink` review, "are we still building the right thing"). Same procedure as slice-boundary; output to `docs/superpowers/reviews/`. Heavy.

### Tool routing — MCP for derived work, Task for synthesis only

`Task`/`Agent` subagents start with **zero cache and are billed against the maintainer's primary Claude budget**. The `claude-review` MCP runs against `claudea` (a Qwen-backed Claude-CLI clone) on a separate path that is **free for the maintainer**. **Default to MCP for any read-only / research / audit task.** Reserve `Task` for work that genuinely requires:

- Code synthesis with Claude-quality output (e.g., implementing a task from an approved slice plan).
- Multi-file editing or write-back to the repo (`Task` can write; MCP tools are read-only).
- Decisions where Claude's specific reasoning is the asset (architectural calls, brainstorm partner work).

Routing rules:

| Task | Tool |
|---|---|
| Read a large file and answer a focused question | `mcp__claude-review__read_with_question` |
| Research a question across the codebase | `mcp__claude-review__research_project` |
| Find code examples by pattern / intent | `mcp__claude-review__find_examples_of` |
| Audit a PR diff against the spec | `mcp__claude-review__audit_pr` |
| Trace a feature through git history | `mcp__claude-review__code_archaeology` |
| Compare two files | `mcp__claude-review__compare_files` |
| Read a past conversation `.jsonl` | `mcp__claude-review__inspect_transcript` |
| Implement task N from a slice plan | `Task(subagent_type="general-purpose", ...)` |
| Two-stage review during SDD | `Task` (per `superpowers:subagent-driven-development`) |
| Brainstorm an architectural decision | direct Claude in plan mode (no subagent) |

If unsure: **try MCP first**. If the MCP tool returns shallow or wrong results, escalate to `Task`. **NEVER** default to `Task` for read-only work — you spend the maintainer's primary budget on something the free tier can do.

### MCP drift-detection toolkit

Three of the MCP tools above carry fixed roles in the audit ritual:

- `mcp__claude-review__audit_pr` — adversarial diff review against the locked spec. **Post-merge.**
- `mcp__claude-review__research_project` — codebase-wide question against north-star + design. **Lightweight audits + ad-hoc drift checks.**
- `mcp__claude-review__inspect_transcript` — extracts human/assistant words from `.jsonl`, stripped of tool noise. **Post-merge** to verify shipped state matches user-stated intent (not just spec).

When Copilot reviewer is rate-limited (or absent), the trio above replaces the second-opinion gate per the PR merge rule.

## Trajectory and anchors

The path to v0.1 is **not** a fixed cascade. Slices are plot points that get added, reordered, or split as alignment audits surface drift. Two **anchors** are non-negotiable for v0.1:

- **Anchor A — Mac shell + ASR sidecar.** First time the product is usable in app form. SwiftUI app under `apps/Panops/`, sidecars `apps/panops-asr-mac/` and `apps/panops-llm-mac/`.
- **Anchor B — Live capture.** ScreenCaptureKit + audio + screenshot sampling. Risk-last surface.

Everything else is composable trajectory toward v0.1, decided slice-by-slice. **NEVER** treat the trajectory list below as commitment — it's current best understanding, amendable at every audit.

### Shipped (history)

1. Skeleton — Cargo workspace, fixtures (audio, video, screenshots).
2. Headless CLI — `AsrProvider` + whisper-rs adapter, JSON segments to stdout.
3. Post-pass + diarization — pyannote via sherpa-rs adapter, speaker labels merged.
4. Notes generation — `LlmProvider` + `NotesGenerator` + `MarkdownExporter`.
5. IPC — JSON-RPC + WebSocket over UDS, Rust test client.

### Current trajectory toward v0.1 (amendable)

- **#74 fix** — real adapters in `panops-engine serve` (eager-after-bind or lazy ctor). Blocks Anchor A.
- **#17 SQLite persistence** — `Storage` port + per-meeting + cross-meeting registry. Blocks Anchor A AND v0.1 acceptance #4.
- **Real-meeting calibration** — #14 ASR threshold, #18 model fallback, #19 diar coverage. Run on a real bilingual meeting. Blocks v0.1 acceptance #5.
- **Anchor A — Mac shell + ASR sidecar.**
- **Anchor B — Live capture.**
- **Real-meeting smoke** — end-to-end real bilingual meeting. Inevitably surfaces issues. Don't pre-empt.
- **Package + sign + notarize** — `scripts/package.sh`, code signing, notarization. v0.1 acceptance #6.
- **v0.1 release** — tag, release notes, public README polish.

The fixed 7-slice cascade in `docs/superpowers/specs/2026-04-30-panops-design.md` is **historical**. This section is the live trajectory; the design spec is locked architecture, not a project plan.

Active slice: see project board milestones.

## Session rituals

### Pickup ritual (run at the START of every session)

1. Read `docs/north-star.md`. Confirm the active slice serves it.
2. Run `git status` and `git log -10 --oneline`.
3. Open the project board. Find the active slice milestone.
4. Read the most recent file in `docs/superpowers/sessions/` for context.
5. Read the active plan in `docs/superpowers/plans/` (latest file not in `done/`).
6. Read the most recent alignment audit in `docs/superpowers/reviews/`. Note any open drift items.
7. **If the session log is >24h old OR a `/compact` happened recently OR ANY of the lightweight-audit triggers fired**: run a lightweight alignment audit (Methodology → Alignment-audit ritual → Lightweight) before proceeding.
8. Identify next action: top-severity open issue in the active milestone with `Status: Todo` and no blocker, OR next unchecked step in the active plan.
9. State explicitly: `"Slice 0N is X/Y done. Last session: <slug>. Next: <issue #N or step>. Drift items open: <count or 'none'>. Blockers: <list or 'none'>."`
10. **MUST** wait for the maintainer's "go" before resuming code changes.

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
3. **Run the slice-boundary alignment audit** (Methodology → Alignment-audit ritual → Slice-boundary). Write findings to `docs/superpowers/reviews/`. This is **non-skippable**.
4. **Re-read `docs/north-star.md`.** If shipped state diverged from it, amend or file blocking debt before queueing the next slice.
5. **Re-read the trajectory list.** Add / remove / reorder slices based on what the audit surfaced. The trajectory is amendable; this is when to amend it.
6. Brainstorm the next trajectory item via the `superpowers:brainstorming` skill, then `superpowers:writing-plans`. **NEVER** queue multiple slice plans ahead.
7. Open a slice tracking issue for the new milestone (label `type:feature`).

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

- **NEVER** propose work outside the active anchor's surface area without a brainstorm. (Live capture before Anchor B is reached, native API bindings before Anchor A, etc.)
- **NEVER** treat the trajectory list as a fixed cascade. Slices iterate; reorder/add/split as alignment audits surface drift.
- **NEVER** add features beyond the active slice's plan.
- **NEVER** pre-trait. One trait when needed, one real impl, one fake.
- **NEVER** open or merge a PR autonomously. The maintainer opens PRs and merges them. (Commits within an approved slice plan are part of L3 work and don't need per-commit approval — they're bounded by the plan's checklist.)
- **NEVER** skip a scheduled alignment audit (lightweight, slice-boundary, or maintainer-triggered). Drift detection is non-optional.
- **NEVER** give time estimates (no "ships in weeks", "X-day project", schedule framing). Compare options on capability/risk/cleanliness.
- **NEVER** phone home. Zero telemetry, ever, even opt-in.
- **NEVER** delete or rewrite files in `docs/superpowers/{specs,plans,sessions,reviews}/` or `docs/research/` without explicit instruction. They're history.
- **NEVER** leave a "deferred" item only in markdown — file it as an issue (see Debt rule).
- **NEVER** mark an issue as "wontfix" via label. Close as "not planned" with a comment instead.

## When in doubt

Priority: **`docs/north-star.md` > active slice spec > design spec > slice plan**. North star is the goal; design and specs are translations; plans iterate. If they disagree, escalate before deciding.

If still unclear after reading the priority chain, run a lightweight alignment audit (Methodology) to surface what's drifted.

## Local additions

Personal notes that shouldn't ship publicly go in `AGENTS.local.md` (gitignored). Keep public-facing rules in this file.
