#!/usr/bin/env python3
"""Belt-and-braces secret redactor for AI review output.

Reads a file path from argv[1], scrubs likely secret material in place, and
writes the result back. Optionally substitutes literal values of env vars
listed in argv[2..] (so the workflow can pass the actual secret values it
sourced from `secrets.X`).

Patterns are intentionally generous on key=value shapes — false positives
are cheap (REDACTED markers in the comment), false negatives are expensive
(real secret leaked into a public PR thread).

The companion test script `test-redact-review.sh` plants every supported
pattern into a fixture, runs this script, and asserts none survive.
"""

import os
import re
import sys

REDACT = "★★★REDACTED★★★"

# Vendor-prefixed tokens — ordered so longer/more-specific prefixes match
# before shorter ones. The `BSA*` prefix here is Brave Search.
VENDOR_PATTERNS = [
    r"sk-ant-[A-Za-z0-9_\-]{50,}",
    r"sk-proj-[A-Za-z0-9_\-]{20,}",
    r"sk-svcacct-[A-Za-z0-9_\-]{20,}",
    r"sk-sp-[A-Za-z0-9]{32,}",
    r"sk-[A-Za-z0-9]{32,}",
    r"ghp_[A-Za-z0-9]{30,}",
    r"gho_[A-Za-z0-9]{30,}",
    r"ghu_[A-Za-z0-9]{30,}",
    r"ghs_[A-Za-z0-9]{30,}",
    r"github_pat_[A-Za-z0-9_]{60,}",
    r"AKIA[A-Z0-9]{16}",
    r"AIza[A-Za-z0-9_\-]{30,}",
    r"xox[baprs]-[A-Za-z0-9\-]{10,}",
    r"hf_[A-Za-z0-9]{30,}",
    r"BSA[A-Za-z0-9_\-]{20,}",
]

# Authorization header with Bearer-style token. Mirrors the local CC hook
# at ~/.claude/scripts/redact-secrets.py — the workflow inline-regex was
# missing this and the test caught it.
BEARER = re.compile(
    r"(?i)(Authorization)\s*:\s*Bearer\s+([A-Za-z0-9._\-]{12,})"
)

# Generic "key=value" / "token: 'value'" shapes (case-insensitive). Looser
# than the vendor list so it catches custom backends and env-style dumps.
GENERIC_KV = re.compile(
    r"""
    (
      (?i:api[_\-]?key|access[_\-]?key|secret[_\-]?key|secret|password|passwd|
          token|bearer|credential|auth(?:orization)?|private[_\-]?key)
    )
    \s* [:=] \s*
    ['"]?
    ([A-Za-z0-9_\-/+=.]{12,})
    ['"]?
    """,
    re.VERBOSE,
)

PEM = re.compile(
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
    re.DOTALL,
)


def redact(text: str, literal_values: list[str]) -> str:
    """Run all redaction passes on a string."""
    # 1. Literal substitution for known secret values from env.
    for v in literal_values:
        if v and len(v) >= 12:
            text = text.replace(v, REDACT)
    # 2. Vendor patterns (most specific shapes).
    for pat in VENDOR_PATTERNS:
        text = re.sub(pat, REDACT, text)
    # 3. Authorization: Bearer ... (must run before GENERIC_KV so the
    #    token is replaced as a unit, not split by the kv regex).
    text = BEARER.sub(lambda m: f"{m.group(1)}: Bearer {REDACT}", text)
    # 4. Generic key=value shapes.
    text = GENERIC_KV.sub(
        lambda m: m.group(0).replace(m.group(2), REDACT), text
    )
    # 5. PEM private-key blocks.
    text = PEM.sub(REDACT, text)
    return text


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(
            "usage: redact-review.py <path> [ENV_VAR ENV_VAR ...]",
            file=sys.stderr,
        )
        return 2

    path = argv[1]
    env_vars = argv[2:]
    literal_values = [os.environ.get(v, "") for v in env_vars]

    try:
        text = open(path).read()
    except FileNotFoundError:
        print(f"redact: input not found: {path}", file=sys.stderr)
        return 0  # don't fail the workflow if upstream produced no output

    open(path, "w").write(redact(text, literal_values))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
