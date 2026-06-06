#!/usr/bin/env bash
# panops project override for 01-runtime-truth.
# Pins to cargo specifically (skip global auto-detection — panops IS
# cargo) and uses the workspace-wide locked-deps build/test that
# AGENTS.md mandates plus clippy parity with CI.

set -uo pipefail
LIB="${AI_REVIEW_LIB:-$HOME/.local/share/ai-review/lib/common.sh}"
# shellcheck disable=SC1090
source "$LIB"

NAME=runtime-truth
WT_DIR="$RUN_DIR/wt"
BUILD_LOG="$RUN_DIR/build.log"
TEST_LOG="$RUN_DIR/test.log"
CLIPPY_LOG="$RUN_DIR/clippy.log"

applies() { diff_touches '.+\.(rs|toml)$'; }

cleanup_worktree() {
  if [ -d "$WT_DIR" ]; then
    git -C "$REPO_ROOT" worktree remove --force "$WT_DIR" 2>/dev/null \
      || rm -rf "$WT_DIR"
  fi
}

run() {
  trap cleanup_worktree EXIT

  echo "  • [$NAME] creating worktree at $WT_DIR ($HEAD_SHA)..." >&2
  if ! git -C "$REPO_ROOT" worktree add --detach "$WT_DIR" "$HEAD_SHA" >/dev/null 2>&1; then
    echo "(worktree create failed)" > "$BUILD_LOG"
    echo "(skipped)" > "$TEST_LOG"
    echo "(skipped)" > "$CLIPPY_LOG"
    build_rc=99; test_rc=skipped; clippy_rc=skipped
  else
    echo "  • [$NAME] cargo build --workspace --locked..." >&2
    ( cd "$WT_DIR" && timeout 300 cargo build --workspace --locked 2>&1 ) > "$BUILD_LOG"
    build_rc=$?

    test_rc="skipped"
    clippy_rc="skipped"
    if [ "$build_rc" -eq 0 ]; then
      echo "  • [$NAME] cargo test --workspace --locked..." >&2
      ( cd "$WT_DIR" && timeout 360 cargo test --workspace --locked 2>&1 ) > "$TEST_LOG"
      test_rc=$?

      # CI parity: panops requires clippy --workspace --all-targets
      # --locked -- -D warnings. Run it locally so a missed clippy
      # warning doesn't surprise the maintainer at PR time.
      echo "  • [$NAME] cargo clippy --workspace --all-targets --locked -- -D warnings..." >&2
      ( cd "$WT_DIR" && timeout 240 cargo clippy --workspace --all-targets --locked -- -D warnings 2>&1 ) > "$CLIPPY_LOG"
      clippy_rc=$?
    else
      echo "(build failed; skipping tests + clippy)" > "$TEST_LOG"
      echo "(build failed; skipping clippy)" > "$CLIPPY_LOG"
    fi
  fi

  echo "  • [$NAME] build_rc=$build_rc test_rc=$test_rc clippy_rc=$clippy_rc; analyzing..." >&2

  local sys; sys="$(stage1_header)
Additional context: this reviewer just executed cargo build + test +
clippy on the PR head in a sandboxed worktree, AND the orchestrator
pre-fetched GitHub Actions CI status for HEAD_SHA.

Local outputs:
  $BUILD_LOG   (cargo build --workspace --locked, exit: $build_rc)
  $TEST_LOG    (cargo test  --workspace --locked, exit: $test_rc)
  $CLIPPY_LOG  (cargo clippy --workspace --all-targets --locked -- -D warnings, exit: $clippy_rc)

CI status:
  $RUN_DIR/ci-status.md
  $RUN_DIR/ci-status.json
  $RUN_DIR/ci-logs/*.log   (per-failed-job, when present)

When local and CI disagree (e.g. local mac passes, CI Linux fails),
the divergence itself is a finding."

  local user="Focus area: RUNTIME TRUTH — panops cargo workspace.

You have actual build/test/clippy output AND the GitHub Actions
verdict. Use them.

What to flag:
- If local cargo build failed: root cause + exact compiler message
  (file:line). Distinguish 'real bug introduced by this PR' from
  'flaky env issue'. panops's CI matrix builds on macOS and Linux
  — if local (macOS) is passing but a Linux warning appeared in
  ci-status.md, treat the Linux signal as truth.
- If local tests failed: each failing test, root cause, what
  assumption broke. panops integration tests under
  \`crates/panops-engine/tests/\` have flaky timing on heavy machines
  — distinguish 'test asserts a real invariant that broke' from
  'wait_for_socket race'.
- If clippy failed: every warning is a CI failure (panops uses
  \`-D warnings\`). Quote each.
- If local passed but CI failed (or vice versa): the divergence is a
  finding. Note: panops CI requires \`cargo clippy --workspace
  --all-targets --locked -- -D warnings\` AND \`cargo fmt --check\`,
  not just build/test.
- New compiler warnings of substance — not style nits (clippy /
  rustfmt own those).
- Test file edited in the diff but produces no observable output
  here (silent-pass risk).
- Slice-N spec promises a method or test, ci-status shows green CI,
  but mapping the diff to test.log shows zero invocations of the new
  surface — flag silent under-coverage.

What NOT to flag:
- Style nits (clippy / rustfmt — those are CI-enforced, not your job).
- Transcription of full logs — quote only the relevant excerpt.
- Findings mechanically obvious from the operator-visible logs —
  focus on synthesis: 'why did this fail?' not 'X failed'.

If everything passed locally and on CI with no warnings of substance,
output: NONE."

  call_claude "$NAME" \
    "Read Glob Grep Bash(cargo *) Bash(git diff *) Bash(git log *) Bash(git show *)" \
    "$sys" "$user" \
    "$RUN_DIR/stage1/$NAME.md" "$RUN_DIR/stage1/$NAME.transcript" 600
}

case "${1:-run}" in
  applies) applies ;;
  run) run ;;
  *) echo "usage: $0 [applies|run]" >&2; exit 2 ;;
esac
