#!/usr/bin/env bash
# panops project override for 05-intent.
# References panops's specific source-of-truth chain.

set -uo pipefail
LIB="${AI_REVIEW_LIB:-$HOME/.local/share/ai-review/lib/common.sh}"
# shellcheck disable=SC1090
source "$LIB"

NAME=intent

applies() { ! diff_is_empty; }

run() {
  local sys; sys="$(stage1_header)"
  local user
  if [ "${AI_REVIEW_MODE:-pr}" = "audit" ]; then
    user="Focus area: INTENT / ALIGNMENT (whole-repo audit) — panops.

Source-of-truth chain (priority order, per AGENTS.md):
  docs/north-star.md
  > docs/superpowers/specs/<latest>-slice-NN-*.md (active slice spec)
  > docs/superpowers/specs/2026-04-30-panops-design.md (locked design)
  > AGENTS.md (workflow contract)
  > .github/copilot-instructions.md
  > README.md

Cross-check the working tree at HEAD against these. Skip silently if
a doc is missing.

Flag:
- Code that contradicts the north-star or a locked design document.
- Slice spec promises not yet shipped (e.g. spec lists a method, no
  test exists for it).
- AGENTS.md \"NEVER\" rules violated by current code.
- README claims that don't match current behavior.
- Stale documentation: CLI flags renamed in code but not in docs;
  removed methods still listed in proto/ipc.md.

What NOT to flag:
- Architectural / SOLID / DRY concerns — those have dedicated reviewers.
- Runtime / CI failures — that's 01-runtime-truth.
- Adversarial / security / failure-mode concerns — that's 04-risk.
- Legitimate amendments where the doc was updated alongside the code."
  else
    user="Focus area: INTENT / ALIGNMENT — panops.

Cross-check the diff against panops's source-of-truth chain.

INPUTS to read (priority order):
- docs/north-star.md (the \"why\" — never violate without a maintainer
  decision recorded with a date stamp)
- The most recent docs/superpowers/specs/<latest>-slice-NN-*.md
  (active slice spec — locked once approved)
- docs/superpowers/specs/2026-04-30-panops-design.md (locked design)
- AGENTS.md (workflow contract — including 'NEVER' lists, three-tier
  ✅/⚠️/🚫 boundaries on each slice spec, debt rule)
- .github/copilot-instructions.md (PR review invariants)
- The PR description at \$RUN_DIR/pr-meta.md
- \$RUN_DIR/related-prs.md — last few merged PRs that touched these
  files. Read this to spot whether the current PR contradicts a
  decision an earlier PR settled, OR re-opens a scope-creep pattern
  that recurs every few PRs.

What to flag:

- Diff that contradicts the active slice spec. Quote the spec line
  and the diff line side by side.
- Diff that contradicts the north-star. (Highest severity — north-
  star changes require an explicit maintainer decision recorded
  with a date stamp; mid-slice drift is forbidden.)
- AGENTS.md \"NEVER\" rule violated (e.g. derived Serialize on a
  domain error type, env var added for user config, OnceLock without
  catch_unwind, slice plan extended beyond approved scope, 'time
  estimates').
- PR description claims that aren't backed by the diff.
- Out-of-scope changes for the active slice — even improvements, if
  they aren't in the slice's three-tier boundaries, are a finding.
  The slice spec's ✅/⚠️/🚫 lists are the source of truth for what's
  in / ask-first / forbidden.
- Missing test coverage for promised behavior — if the slice spec
  lists a method or a test, find the matching test file. If none,
  flag.
- Cross-PR drift: this PR changes something an earlier PR (in
  related-prs.md) explicitly decided. Cite the earlier PR number.
- Recurring scope-creep across slices: if related-prs.md shows the
  same kind of drift in 2+ recent PRs, surface as ONE finding so the
  maintainer can decide whether the slice boundaries themselves
  need re-drawing.
- \"Deferred\" / \"out of scope\" / \"later slice\" items mentioned
  in spec or PR description that don't have a corresponding GitHub
  issue (per AGENTS.md debt rule).

What NOT to flag:
- Architectural / layer issues — that's 02-architecture.
- Duplication / dead code — that's 03-dryness.
- Failure-mode / security — that's 04-risk.
- A diff that updates BOTH code and the documentation describing it
  (legitimate amendment; not drift).
- A documented spec amendment with a date stamp (deliberate change
  via the proper channel)."
  fi

  call_claude "$NAME" "$STAGE1_TOOLS" "$sys" "$user" \
    "$RUN_DIR/stage1/$NAME.md" "$RUN_DIR/stage1/$NAME.transcript" 600
}

case "${1:-run}" in
  applies) applies ;;
  run) run ;;
  *) echo "usage: $0 [applies|run]" >&2; exit 2 ;;
esac
