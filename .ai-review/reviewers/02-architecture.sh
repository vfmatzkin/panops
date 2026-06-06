#!/usr/bin/env bash
# panops project override for 02-architecture.
# Carries panops-specific hex-arch invariants the global default
# can't assume. Replaces the global by basename match.

set -uo pipefail
LIB="${AI_REVIEW_LIB:-$HOME/.local/share/ai-review/lib/common.sh}"
# shellcheck disable=SC1090
source "$LIB"

NAME=architecture

# panops is a Rust workspace + Swift sidecars (slice 06+). Audit only
# code-shaped files; ignore docs / fixtures / generated.
applies() { diff_touches '.*\.(rs|swift|toml)$'; }

run() {
  local sys; sys="$(stage1_header)"
  local user="Focus area: ARCHITECTURE / LAYER DISCIPLINE — panops invariants.

Read AGENTS.md, docs/north-star.md, the most recent
docs/superpowers/specs/<latest>-slice-NN-*.md, and
docs/superpowers/specs/2026-04-30-panops-design.md. Source-of-truth
priority: north-star > active slice spec > design spec > AGENTS.md.

panops invariants (\"NEVER\" rules — quote verbatim in findings):

- panops-core MUST stay platform-free. Allowed deps: serde, thiserror,
  chrono, tracing, hound, sha2, serde_json, rayon. NO tokio /
  reqwest / whisper-rs / sherpa-rs / rusqlite / cocoa / objc.
  panops-portable holds Rust adapters; panops-mac is
  \`#[cfg(target_os = \"macos\")]\` only.
- panops-protocol is transport-only — no engine logic, no IO, kept
  chrono-free (the comment at \`crates/panops-protocol/src/methods.rs:107\`
  explains: \`started_at\` is \`String\` so non-Rust consumers don't
  need a Rust-specific time crate).
- Domain error types (AsrError, DiarError, LlmError, NotesError,
  StorageError, ExportError) MUST NOT derive serde::Serialize.
  Ratified in commit \`c2e5f34\` / PR #104. Transport conversion lives
  in panops-protocol behind \`domain-conversions\` feature flag.
- One trait at a time + one real impl + one fake. NEVER pre-trait
  for hypothetical future adapters.
- Every port has a conformance harness in \`panops-core::conformance::*\`
  that BOTH the real impl and the fake pass. New ports must ship the
  harness in the same PR.
- OnceLock / OnceCell slots a handler can observe MUST reach a
  terminal Ok / Err on every path including panic. Wrap initializing
  closures in \`std::panic::catch_unwind\` and convert panic payloads
  to Err. Precedent: commit \`02559a3\`. If you see a OnceLock set
  inside a closure that can panic without catch_unwind, flag it.
- New \`pub\` items widen the workspace API surface — flag any new
  \`pub\` (or \`pub(crate)\` upgrade) without justification in the
  PR description.

Concrete things to flag:
- Adapter that bypasses its own port (calls a sibling adapter
  directly instead of going through the trait).
- New port without a fake or conformance harness.
- New real impl without the port abstraction sitting behind it.
- Public-API widening without rationale.
- Test for a new architectural surface MISSING — if a new port /
  trait / public type lands without a conformance test or integration
  test, flag it. (Test-coverage gaps for the architectural change
  are this reviewer's responsibility; there is no separate 'tests'
  reviewer.)
- panops-mac code reachable from non-mac builds.
- IPC types with chrono fields (must stay String at the wire).

Use mcp__claude-review__find_examples_of when checking whether a
pattern matches existing project conventions.

If \$RUN_DIR/related-prs.md is non-empty, scan it for prior PRs that
touched the same architectural boundaries — recurring violations of
the same rule are worth ONE finding referencing PR numbers, not
separate findings each round.

What NOT to flag:
- Internal implementation choices that don't cross a boundary.
- Style nits — clippy / fmt handle those.
- Duplication / dead code — that's 03-dryness.
- Race / cleanup / cancellation — that's 04-risk."

  call_claude "$NAME" "$STAGE1_TOOLS" "$sys" "$user" \
    "$RUN_DIR/stage1/$NAME.md" "$RUN_DIR/stage1/$NAME.transcript" 600
}

case "${1:-run}" in
  applies) applies ;;
  run) run ;;
  *) echo "usage: $0 [applies|run]" >&2; exit 2 ;;
esac
