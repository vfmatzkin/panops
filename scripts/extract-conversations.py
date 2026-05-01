#!/usr/bin/env python3
"""Extract thin user/assistant conversation history from Claude Code session logs.

Reads jsonl files from ~/.claude/projects/-Users-fran-Code-panops/ and
~/.claude/projects/-Users-fran-Code/ (only sessions mentioning "panops"),
extracts user messages + assistant text turns, drops tool calls / tool results
/ system reminders / large code blocks, and writes one markdown file per
session to docs/superpowers/conversations/.

Idempotent: skips sessions whose source mtime hasn't changed since the last
extraction (recorded inline in the output's frontmatter).

Run manually or via the lefthook post-commit hook.
"""
from __future__ import annotations

import json
import re
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
OUT_DIR = HERE / "docs" / "superpowers" / "conversations"

CLAUDE_PROJECTS = Path.home() / ".claude" / "projects"
SOURCE_DIRS = [
    CLAUDE_PROJECTS / "-Users-fran-Code-panops",
    CLAUDE_PROJECTS / "-Users-fran-Code",
]

# Skip sessions in -Users-fran-Code that don't mention panops.
PANOPS_FILTER_DIRS = {CLAUDE_PROJECTS / "-Users-fran-Code"}

CODE_BLOCK_THRESHOLD_LINES = 30  # collapse code blocks longer than this
LONG_TEXT_THRESHOLD_CHARS = 6000  # collapse very long single text blocks

SYSTEM_REMINDER_RE = re.compile(r"<system-reminder>.*?</system-reminder>", re.DOTALL)
LOCAL_COMMAND_RE = re.compile(r"<local-command-caveat>.*?</local-command-caveat>", re.DOTALL)
COMMAND_NAME_RE = re.compile(r"<command-name>.*?</command-name>", re.DOTALL)
COMMAND_MESSAGE_RE = re.compile(r"<command-message>.*?</command-message>", re.DOTALL)
COMMAND_ARGS_RE = re.compile(r"<command-args>.*?</command-args>", re.DOTALL)


@dataclass
class Turn:
    role: str  # "user" or "assistant"
    text: str
    timestamp: str | None


def clean_text(s: str) -> str:
    s = SYSTEM_REMINDER_RE.sub("", s)
    s = LOCAL_COMMAND_RE.sub("", s)
    s = COMMAND_NAME_RE.sub("", s)
    s = COMMAND_MESSAGE_RE.sub("", s)
    s = COMMAND_ARGS_RE.sub("", s)
    s = re.sub(r"\n{3,}", "\n\n", s)
    return s.strip()


def collapse_long_code(text: str) -> str:
    """Replace fenced code blocks longer than threshold with a one-line note."""
    out = []
    in_block = False
    block_lines: list[str] = []
    fence: str | None = None
    for line in text.split("\n"):
        stripped = line.lstrip()
        if not in_block:
            m = re.match(r"^(```+|~~~+)([^\s]*)?", stripped)
            if m:
                in_block = True
                fence = m.group(1)
                block_lines = [line]
                continue
            out.append(line)
        else:
            block_lines.append(line)
            if fence is not None and stripped.startswith(fence):
                if len(block_lines) > CODE_BLOCK_THRESHOLD_LINES:
                    head = "\n".join(block_lines[:3])
                    out.append(head)
                    out.append(f"... ({len(block_lines) - 4} lines elided) ...")
                    out.append(block_lines[-1])
                else:
                    out.extend(block_lines)
                in_block = False
                fence = None
                block_lines = []
    if in_block:
        out.extend(block_lines)
    return "\n".join(out)


def extract_text_from_content(content) -> str:
    """Pull text out of an Anthropic message content list, dropping tool_use / tool_result."""
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""
    parts: list[str] = []
    for b in content:
        if not isinstance(b, dict):
            continue
        t = b.get("type")
        if t == "text":
            parts.append(b.get("text", ""))
        # tool_use, tool_result, image: skip
    return "\n".join(parts)


def session_mentions_panops(path: Path) -> bool:
    """Stream the file in chunks; the keyword may appear late."""
    try:
        with open(path, "rb") as f:
            tail = b""
            while True:
                chunk = f.read(1 << 20)
                if not chunk:
                    return False
                if b"panops" in (tail + chunk).lower():
                    return True
                tail = chunk[-7:]  # carry across chunk boundary (len("panops") = 6)
    except Exception:
        return False


def parse_session(path: Path) -> tuple[list[Turn], str | None]:
    """Return (turns, started_at_iso)."""
    turns: list[Turn] = []
    started_at: str | None = None
    for line in path.open():
        try:
            r = json.loads(line)
        except Exception:
            continue
        ts = r.get("timestamp")
        if ts and started_at is None:
            started_at = ts
        rtype = r.get("type")
        if rtype not in ("user", "assistant"):
            continue
        msg = r.get("message", {})
        text = extract_text_from_content(msg.get("content"))
        text = clean_text(text)
        if not text:
            continue
        if rtype == "user":
            # Skip user turns that are pure tool_result envelopes (caught above)
            # Skip slash-command invocations that are just system payloads.
            if text.startswith("<") and text.endswith(">"):
                continue
            if "tool_use_id" in text[:200]:
                continue
        text = collapse_long_code(text)
        if len(text) > LONG_TEXT_THRESHOLD_CHARS:
            text = text[:LONG_TEXT_THRESHOLD_CHARS] + f"\n\n... ({len(text) - LONG_TEXT_THRESHOLD_CHARS} chars elided) ..."
        turns.append(Turn(role=rtype, text=text, timestamp=ts))
    return turns, started_at


def output_path_for(session_id: str, started_at: str | None) -> Path:
    if started_at:
        try:
            dt = datetime.fromisoformat(started_at.replace("Z", "+00:00"))
            date = dt.strftime("%Y-%m-%d")
        except Exception:
            date = "unknown-date"
    else:
        date = "unknown-date"
    short = session_id.split("-")[0]
    return OUT_DIR / f"{date}-{short}.md"


def render(turns: list[Turn], session_id: str, source_path: Path, started_at: str | None) -> str:
    lines = [
        "---",
        f"session_id: {session_id}",
        f"source: {source_path}",
        f"source_mtime: {datetime.fromtimestamp(source_path.stat().st_mtime, tz=timezone.utc).isoformat()}",
        f"started_at: {started_at or 'unknown'}",
        f"turns: {len(turns)}",
        "---",
        "",
    ]
    for t in turns:
        ts = (t.timestamp or "")[:19]
        marker = "**user**" if t.role == "user" else "**assistant**"
        prefix = f"### {marker}  {ts}".rstrip()
        lines.append(prefix)
        lines.append("")
        lines.append(t.text)
        lines.append("")
        lines.append("---")
        lines.append("")
    return "\n".join(lines)


def existing_source_mtime(out_path: Path) -> str | None:
    if not out_path.exists():
        return None
    seen_first_fence = False
    with out_path.open() as f:
        for line in f:
            stripped = line.strip()
            if stripped == "---":
                if seen_first_fence:
                    return None  # end of frontmatter, not found
                seen_first_fence = True
                continue
            if not seen_first_fence:
                continue
            if line.startswith("source_mtime:"):
                return line.split(":", 1)[1].strip()
    return None


def main() -> int:
    if not OUT_DIR.exists():
        OUT_DIR.mkdir(parents=True)
    written = skipped = scanned = 0
    for src_dir in SOURCE_DIRS:
        if not src_dir.exists():
            continue
        for jsonl in sorted(src_dir.glob("*.jsonl")):
            scanned += 1
            if src_dir in PANOPS_FILTER_DIRS and not session_mentions_panops(jsonl):
                continue
            session_id = jsonl.stem
            turns, started_at = parse_session(jsonl)
            if not turns:
                continue
            out = output_path_for(session_id, started_at)
            current_mtime = datetime.fromtimestamp(
                jsonl.stat().st_mtime, tz=timezone.utc
            ).isoformat()
            prev = existing_source_mtime(out)
            if prev == current_mtime:
                skipped += 1
                continue
            out.write_text(render(turns, session_id, jsonl, started_at))
            written += 1
    print(f"extract-conversations: scanned={scanned} written={written} skipped={skipped} -> {OUT_DIR}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
