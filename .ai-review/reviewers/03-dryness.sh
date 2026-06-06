#!/usr/bin/env bash
# panops project override for 03-dryness.
# Rust-specific duplication / dead-code / rewrite-cycle signals.

set -uo pipefail
LIB="${AI_REVIEW_LIB:-$HOME/.local/share/ai-review/lib/common.sh}"
# shellcheck disable=SC1090
source "$LIB"

NAME=dryness

applies() { diff_touches '.*\.(rs|swift)$'; }

run() {
  local sys; sys="$(stage1_header)"
  local user="Focus area: DRY / SOLID / DEAD CODE / REWRITE CYCLES — panops.

Slices iterate; we want trajectory to converge, not re-litigate.

DRY (duplication):
- New function / struct / type / pattern duplicating one in the
  codebase. Use mcp__claude-review__find_examples_of to confirm.
  Cite both sites: 'duplicated at <crate>/<path>:<line>'.
- N>=2 sites that should clearly share a helper but each have their
  own copy. Common in panops: error-mapping closures, path-canonicalize
  bits, jsonrpsee handler boilerplate, spawn_blocking wrap patterns.
- Same regex / numeric constant / SQL fragment / wire-shape literal
  repeated across crates where a named const in panops-core or
  panops-protocol would be cleaner.

SOLID:
- Trait with so many methods no impl uses them all (Interface-
  Segregation: split into focused traits).
- Struct / module with dual responsibility (single-responsibility:
  flag the 'and' smell).
- match on a kind / type field that should be polymorphism (Open-
  Closed: every new variant requires editing the match arm).
- Adapter reaching into another adapter's concrete type instead of
  the trait (Dependency-Inversion).
- Trait impls violating the trait's documented contract (Liskov:
  conformance harness should catch this — flag if a new impl was
  added without exercising the harness).

DEAD / UNREACHABLE:
- New public functions / structs without callers in the diff.
  Fixture-only callers (tests) flagged as 'fixture-only — keep?'.
- Existing items whose only caller is removed in this diff — they
  become dead, should be removed in the same PR.
- Match arms / branches that can never fire (impossible discriminants).
- \`pub\` items in a module no longer referenced from \`lib.rs\` —
  candidate for \`pub(crate)\` or removal.
- Imports introduced but no longer needed after the diff.

REWRITE CYCLES (panops trajectory health):
- Use mcp__claude-review__code_archaeology on EACH non-trivial file
  in the diff. If a file has been substantially rewritten >=2 times
  in recent history (different commits, different approach), the
  diff entering rewrite #3 is a yellow flag — surface it with the
  prior commit SHAs.
- Check \$RUN_DIR/related-prs.md: are there prior PRs whose diffs
  touched the same hunks? If so, the project is paying repeated
  cost for an unsettled API. Surface as ONE finding referencing PR
  numbers — recurring drift is a slice-spec issue, not per-PR code.
- Particular signal: same crate's \`lib.rs\` re-exports churn across
  slices, OR same trait's signature changes >1 time recently.
  Either suggests the abstraction hasn't settled.

Test gaps for new shared modules:
- If you flag duplication that should be extracted, also flag whether
  the proposed common module would have testable seams. Don't flag
  missing tests for inline duplication — not actionable until
  extraction lands.

What NOT to flag:
- Rust trait impl bodies required by the trait (boilerplate).
- Generated code (build.rs output, vendored deps).
- Tests in tests/fixtures/ (meant to be redundant with conformance
  harnesses by design).
- Style nits — clippy / fmt handle those.
- Architectural / layer violations — that's 02-architecture.

Be conservative: a finding here should be \`extract-this\`-actionable
with both sites and a sketch of the common shape (or a clear case
that this PR is the wrong place to extract)."

  call_claude "$NAME" "$STAGE1_TOOLS" "$sys" "$user" \
    "$RUN_DIR/stage1/$NAME.md" "$RUN_DIR/stage1/$NAME.transcript" 600
}

case "${1:-run}" in
  applies) applies ;;
  run) run ;;
  *) echo "usage: $0 [applies|run]" >&2; exit 2 ;;
esac
