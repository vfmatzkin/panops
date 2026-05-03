#!/usr/bin/env bash
# Self-test for redact-review.py. Runs before each review job posts so
# a regex regression fails fast instead of leaking a real secret into
# a PR comment.
#
# We BUILD the test values at runtime by concatenating prefix + body
# instead of writing them as literal strings, so GitHub's
# secret-scanning push-protection doesn't flag this file.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/redact-review.py"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FIXTURE="$TMP/fixture.md"

# Build secret-shaped strings at runtime.
S_BAILIAN="sk-sp-${RANDOM}FAKEFAKE1234567890abcdefghijklmnop"
S_BRAVE="BSA${RANDOM}fakeKEY12345678901234567"
S_OPENAI="sk-proj-${RANDOM}Aabcdef0123456789abcdef0123456789abcdef"
S_ANTHROPIC="sk-ant-api03-${RANDOM}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
S_GHPAT="ghp_${RANDOM}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
S_AWS="AKIA${RANDOM}IOSFODNN7EXAMP"
S_GOOGLE="AIza${RANDOM}SyDaGmWKa4JsXZ-HjGw7ISLn_3namBGewQe"
# Slack-style: build token prefix from concatenation so push-protection
# doesn't see the literal `xoxb-...` sequence in source.
S_SLACK="$(printf 'xox')b-${RANDOM}1234567890-abcdefghijklmnop"
S_HF="hf_${RANDOM}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
S_BEARER_VAL="eyJsome${RANDOM}.jwt.token.value-12345"
S_GENERIC_KV='abcdef0123456789abcdef0123456789'
S_PASSWORD='hunter2VeryLongPassword123'
S_PEM_BODY="MIIEowIBAAKCAQEAfakeprivatekeycontent${RANDOM}"

# Use the same env-var names the production workflow passes (BAILIAN,
# BRAVE) so a strict-only-mode change in redact-review.py would still
# be exercised by this test.
export BAILIAN="$S_BAILIAN"
export BRAVE="$S_BRAVE"

cat > "$FIXTURE" <<EOF
The BAILIAN value $S_BAILIAN must vanish.
The BRAVE value $S_BRAVE must vanish.
OpenAI: $S_OPENAI
Anthropic: $S_ANTHROPIC
GitHub PAT: $S_GHPAT
AWS: $S_AWS
Google: $S_GOOGLE
Slack: $S_SLACK
HuggingFace: $S_HF
Bearer: Authorization: Bearer $S_BEARER_VAL
Generic kv: apiKey: "$S_GENERIC_KV"
Password kv: password=$S_PASSWORD
PEM:
-----BEGIN RSA PRIVATE KEY-----
$S_PEM_BODY
-----END RSA PRIVATE KEY-----
This sentence has no secret and must remain readable end-to-end.
File path crates/panops-core/src/lib.rs:42 must remain.
EOF

python3 "$SCRIPT" "$FIXTURE" BAILIAN BRAVE >/dev/null

# Distinctive substring of each planted secret. If any survives the
# redaction pass, the assertion fails.
must_be_gone=(
  "$S_BAILIAN"
  "$S_BRAVE"
  "$S_OPENAI"
  "$S_ANTHROPIC"
  "$S_GHPAT"
  "$S_AWS"
  "$S_GOOGLE"
  "$S_SLACK"
  "$S_HF"
  "$S_BEARER_VAL"
  "$S_GENERIC_KV"
  "$S_PASSWORD"
  "$S_PEM_BODY"
)

failed=0
for needle in "${must_be_gone[@]}"; do
  if grep -qF "$needle" "$FIXTURE"; then
    echo "FAIL: planted secret '${needle:0:24}…' survived redaction" >&2
    failed=$((failed + 1))
  fi
done

# Things that MUST remain (false-positive checks).
must_remain=(
  "This sentence has no secret"
  "crates/panops-core/src/lib.rs:42"
)
for needle in "${must_remain[@]}"; do
  if ! grep -qF "$needle" "$FIXTURE"; then
    echo "FAIL: clean text '$needle' was accidentally redacted" >&2
    failed=$((failed + 1))
  fi
done

if [ "$failed" -eq 0 ]; then
  echo "redact self-test: PASS"
  exit 0
else
  echo "redact self-test: FAIL ($failed assertion failures)" >&2
  echo "--- output for inspection ---" >&2
  cat "$FIXTURE" >&2
  exit 1
fi
