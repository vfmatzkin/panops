#!/usr/bin/env bash
# panops project override for 04-risk.
# Rust-specific failure modes + panops-specific security invariants.

set -uo pipefail
LIB="${AI_REVIEW_LIB:-$HOME/.local/share/ai-review/lib/common.sh}"
# shellcheck disable=SC1090
source "$LIB"

NAME=risk

applies() { ! diff_is_empty; }

run() {
  local sys; sys="$(stage1_header)"
  local user="Focus area: ADVERSARIAL RISK — panops.

Failure modes + security in one focus. Imagine an unfriendly
environment: flaky network, attacker-controlled inputs, processes
killed mid-operation, full filesystems. What in this diff breaks?

FAILURE-MODE concerns (Rust-specific shapes):
- Unhandled errors: ignored \`Result\`, \`unwrap()\` on a value that
  can be runtime-failure (not compile-time-impossible), \`?\` on a
  call whose error variant the caller can't actually handle.
- Lost wakeups, ordering across async tasks. tokio: \`Notify\` used
  where \`watch\` is needed (pre-PR-91 lesson — \`watch\` stores
  current value, \`Notify\` doesn't).
- OnceLock / OnceCell slots NOT reaching a terminal state on every
  path including panic. AGENTS.md rule (commit 02559a3): wrap
  initializing closures in \`std::panic::catch_unwind\` and convert
  panic payloads to Err so the slot never stays permanently None.
  If you see a OnceLock set inside a panicking closure without
  catch_unwind, flag it.
- \`spawn_blocking\` JoinHandle dropped without an awaiter — panics
  silently swallowed. Slice-05 lesson.
- Resource cleanup on early-return / panic. Sockets / file handles /
  temp dirs / DB connections must release on every path. Drop helps,
  but \`std::mem::forget\` and \`std::process::exit\` bypass it.
- Cancellation safety: a tokio task dropped mid-flight — is the
  in-flight work observable in a half-applied state? IPC handlers
  particularly.
- Panic propagation across thread / task / process boundaries. A
  rayon worker panicking — does the join-handle observe it?

SECURITY concerns (panops invariants):
- Untrusted input flowing to a sensitive sink: shell exec, SQL
  string-formatting (use \`rusqlite::params!\` for parameterized
  queries), template engines.
- Path traversal in file handling. Slice-05 lesson: any
  \`PathBuf::from(<contributor-controlled>)\` MUST be canonicalized
  + bounds-checked against an allowlist before use. The IPC
  \`notes.generate\` handler does this; flag any new handler that
  accepts a path without similar treatment.
- Secret-shape literals in source / tests / logs: API keys, Bearer
  tokens, JWTs, OLLAMA_HOST URLs with credentials, *.pem fragments.
- Internal error messages leaking filesystem / SQL / version detail
  to the wire. Slice-05 hardening pattern: opaque message externally
  (\`\"internal error\"\`), full detail to \`tracing::error!\`. Any
  new \`IpcError::Internal { message: ... }\` whose message embeds
  a path or SQL fragment is a finding.
- Phone-home behavior — NEVER allowed (genesis charter, AGENTS.md,
  north-star). Any new outbound HTTP / DNS to infrastructure that
  isn't an explicitly-stated dep is a finding.
- Env-var-based USER config (per AGENTS.md \"no env vars for user
  config\"). Env vars are last-resort dev/CI escape hatches. New
  PANOPS_* env vars without spec call-out are a finding.

Untested failure modes:
- If the diff adds an error path with no test exercising it, flag
  it. This reviewer's responsibility — there is no separate test
  reviewer.

Be concrete: name the input source, the sink it can reach, and what
happens at each step. Adversarial reasoning, not vague cautioning.

ci-status.md may show test failures that are themselves runtime-
failure findings — but if 01-runtime-truth covers them, don't
duplicate. Focus on what's NOT failing yet but COULD.

What NOT to flag:
- Style nits — clippy handles those.
- Architectural placement — that's 02-architecture.
- Duplication / dead code — that's 03-dryness.
- General possibility of failure without a concrete trigger — say
  'X is unhandled' only when you can name an input or condition that
  causes X."

  call_claude "$NAME" "$STAGE1_TOOLS" "$sys" "$user" \
    "$RUN_DIR/stage1/$NAME.md" "$RUN_DIR/stage1/$NAME.transcript" 600
}

case "${1:-run}" in
  applies) applies ;;
  run) run ;;
  *) echo "usage: $0 [applies|run]" >&2; exit 2 ;;
esac
