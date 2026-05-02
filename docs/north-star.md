# panops — North Star

**Status**: Append-only. Amended only at major-milestone boundaries (v0.1 → v0.2, etc.) with explicit maintainer decision. Never edited mid-slice.

## What panops is

> Open-source local-first macOS recorder with screenshot-anchored, multilingual meeting notes. Native Mac UI + portable backend. Zero telemetry. MIT licensed. Hex/screaming arch. SOLID/DRY.

That sentence is a compressed restatement of the maintainer's genesis charter (verbatim source: `docs/superpowers/conversations/2026-04-30-f0690f89.md:53`). It is the highest-priority source of truth in the repo, above any spec, plan, or AGENTS.md rule.

## Why it exists

- **For the maintainer first.** A tool he'll actually use for his own bilingual (EN/ES) meetings, on his own Mac. Dogfooding is the validation.
- **For other devs second.** A reference for hex-arch Rust + Mac-native sidecars + portable adapters. The CLI is a dev/CI surface, not the product.
- **Open-source.** No monetisation, no telemetry, no phoning home. MIT.

## v0.1 acceptance criteria

v0.1 is **not** "all 7 slices shipped." It's **a Mac app the maintainer would actually use for his next meeting.** Six observable criteria:

1. ☐ Open the Mac app, hit "record", run a real bilingual meeting → audio + screenshots captured.
2. ☐ Stop recording → diarized transcript appears in the app.
3. ☐ "Generate notes" → markdown file with frontmatter, sections with screenshots, action items, no narrative/key-points duplication.
4. ☐ Notes file persists across app restarts (SQLite + on-disk markdown).
5. ☐ Output passes the maintainer's *"would I actually use this for my next meeting?"* test on a real bilingual meeting.
6. ☐ Build + sign + notarize the `.app`; runs on a clean Mac with no dev tools installed.

Criterion #5 is the only one not mechanically verifiable — it's the gate that prevents shipping technically-passing-but-actually-bad notes. The maintainer runs a real meeting, reads the output, and decides.

## What v0.1 is NOT

- Not multi-user.
- Not cloud-synced.
- Not Notion/Slack/Obsidian export (filed as debt; future).
- Not Linux or Windows.
- Not iOS / iPad.
- Not real-time streaming UI (live transcript shows during recording is fine; live partials are bonus, not required).
- Not a Notion-style enhanced-markdown viewer (the markdown gets WRITTEN as `NotionEnhanced` dialect by default; opening it elsewhere is the user's problem).

## Anchors (non-negotiable for v0.1)

Two architectural surfaces that must exist for v0.1 to be v0.1:

- **Anchor A — Mac shell + ASR sidecar.** SwiftUI app + WhisperKit / FluidAudio sidecars. Without this, panops is a CLI for devs, not a product.
- **Anchor B — Live capture.** ScreenCaptureKit + audio + screenshot sampling. Without this, criteria #1-2 are unmet.

Anchors block v0.1. Trajectory slices toward them are amendable (see `AGENTS.md` → Trajectory and anchors).

## Constraints inherited from the genesis charter

These are absolute. They apply to every slice. Any drift away from them is a north-star violation.

- **Multilingual day 1**, with a per-meeting language toggle. Bilingual EN/ES is the canonical test.
- **Native Mac UI** (SwiftUI) + **portable backend** (Rust hex). Not Tauri, not Electron.
- **Zero telemetry** ever, even opt-in.
- **Hex / screaming architecture.** `panops-core` has no platform deps; `panops-mac` is `#[cfg(target_os="macos")]`; `panops-portable` holds the Rust adapters; `panops-protocol` holds wire types.
- **SOLID / DRY.** Visible in port/adapter discipline, fixture-as-adapter, conformance harness per port.
- **One trait at a time + one real + one fake.** Never pre-trait.
- **MIT** licensed. No co-author attribution on commits.
- **No env vars for user config.** Auto-detect via macOS; env vars are last-resort dev/CI escape hatches and must be flagged in the spec.

## How this doc gets amended

- **At v0.1 ship**: rewrite the v0.1 acceptance section as v0.1-shipped (timestamped) and add a v0.2 acceptance section.
- **If a slice surfaces a constraint that conflicts with a north-star item**: that's a *blocking* alignment-audit finding. Amendment requires a maintainer decision recorded in this file with a date stamp. Mid-slice silent amendments are forbidden.
- **If trajectory shifts** (a slice gets added/removed/reordered): amend `AGENTS.md` → Trajectory and anchors, NOT this file. Trajectory is amendable; the goal isn't.

Last amended: 2026-05-02 (initial).
