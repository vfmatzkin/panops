//! Default `NotesExporter`. Writes `<dest>/notes.md` and a sibling
//! `screenshots/` directory of copied images.

use std::fs;
use std::path::{Path, PathBuf};

use panops_core::exporter::{ExportArtifact, ExportError, NotesExporter};
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::ir::{NotesSection, Screenshot, StructuredNotes};

pub struct MarkdownExporter;

impl NotesExporter for MarkdownExporter {
    fn export(&self, notes: &StructuredNotes, dest: &Path) -> Result<ExportArtifact, ExportError> {
        if dest.exists() && !dest.is_dir() {
            return Err(ExportError::InvalidDest(format!(
                "{dest:?} exists but is not a directory"
            )));
        }
        if !dest.exists() {
            fs::create_dir_all(dest)?;
        }
        let screenshots_dir = dest.join("screenshots");
        let mut assets: Vec<PathBuf> = Vec::new();

        let mut body = String::new();
        body.push_str(&render_frontmatter(notes));
        body.push('\n');
        for sec in &notes.sections {
            body.push_str(&render_section(
                sec,
                notes.frontmatter.dialect,
                &screenshots_dir,
                &mut assets,
            )?);
            body.push_str("\n---\n\n");
        }
        if body.ends_with("\n---\n\n") {
            body.truncate(body.len() - "\n---\n\n".len());
            body.push('\n');
        }

        let primary = dest.join("notes.md");
        fs::write(&primary, body)?;
        Ok(ExportArtifact {
            primary_file: primary,
            assets,
        })
    }
}

/// Humanize a raw speaker ID string (`speaker_N`) to `Speaker N+1`.
/// Returns the original string if it doesn't match the `speaker_N` pattern.
fn humanize_speaker_id(raw: &str) -> String {
    if let Some(num) = raw.strip_prefix("speaker_") {
        if let Ok(id) = num.parse::<u32>() {
            return format!("Speaker {}", id.saturating_add(1));
        }
    }
    raw.to_string()
}

/// Replace all `speaker_N` patterns in text with `Speaker N+1`.
/// Used for narrative content where speaker references may appear.
fn humanize_speakers_in_text(text: &str) -> String {
    // Replace each `speaker_N` with `Speaker N+1`. `match_indices` on the
    // ASCII pattern yields only valid UTF-8 char boundaries, so this stays
    // safe on multi-byte text (e.g. accented Spanish) — the old
    // byte-increment scan panicked when it sliced mid-character.
    let mut result = String::with_capacity(text.len());
    let mut last = 0;
    for (start, _) in text.match_indices("speaker_") {
        if start < last {
            continue; // inside a region already consumed
        }
        result.push_str(&text[last..start]);
        let num_start = start + "speaker_".len();
        let digits_len = text[num_start..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if digits_len > 0 {
            if let Ok(id) = text[num_start..num_start + digits_len].parse::<u32>() {
                result.push_str(&format!("Speaker {}", id.saturating_add(1)));
                last = num_start + digits_len;
                continue;
            }
        }
        // Not a valid `speaker_N`: keep the literal prefix and continue past it.
        result.push_str("speaker_");
        last = num_start;
    }
    result.push_str(&text[last..]);
    result
}

fn render_frontmatter(notes: &StructuredNotes) -> String {
    let fm = &notes.frontmatter;
    let mut s = String::from("---\n");
    s.push_str(&format!("title: {}\n", yaml_scalar(&fm.title)));
    s.push_str(&format!("date: {}\n", fm.date.format("%Y-%m-%d")));
    s.push_str(&format!("started_at: {}\n", fm.started_at.to_rfc3339()));
    s.push_str(&format!("duration_ms: {}\n", fm.duration_ms));
    s.push_str(&format!(
        "languages: {}\n",
        yaml_list(&notes.frontmatter.languages)
    ));
    if fm.speakers.is_empty() {
        s.push_str("speakers: []\n");
    } else {
        s.push_str("speakers:\n");
        for sp in &fm.speakers {
            // Humanize speaker IDs at render boundary: speaker_0 → Speaker 1
            s.push_str(&format!("  - {}\n", yaml_scalar(&humanize_speaker_id(sp))));
        }
    }
    if fm.tags.is_empty() {
        s.push_str("tags: []\n");
    } else {
        s.push_str("tags:\n");
        for tag in &fm.tags {
            s.push_str(&format!("  - {}\n", yaml_scalar(tag)));
        }
    }
    s.push_str(&format!("template: {}\n", yaml_scalar(&fm.template)));
    s.push_str(&format!("dialect: {}\n", fm.dialect.as_str()));
    s.push_str(&format!(
        "panops_version: {}\n",
        yaml_scalar(&fm.panops_version)
    ));
    if let Some(p) = &fm.source_audio {
        s.push_str(&format!(
            "source_audio: {}\n",
            yaml_scalar(&p.display().to_string())
        ));
    }
    s.push_str("---\n");
    s
}

fn render_section(
    sec: &NotesSection,
    dialect: MarkdownDialect,
    screenshots_dir: &Path,
    assets: &mut Vec<PathBuf>,
) -> Result<String, ExportError> {
    let mut s = String::new();
    s.push_str(&format!("## {}. {}\n\n", sec.index, sec.title));
    s.push_str(&format!(
        "*[{} – {}]*\n\n",
        format_mmss(sec.time_range_ms.0),
        format_mmss(sec.time_range_ms.1)
    ));
    // Humanize any speaker_N references in narrative (e.g., fallback transcript dumps)
    s.push_str(&humanize_speakers_in_text(sec.narrative_md.trim()));
    s.push_str("\n\n");
    if !sec.key_points.is_empty() {
        s.push_str("**Key points:**\n");
        for kp in &sec.key_points {
            // Key points shouldn't normally have speaker refs, but humanize if present
            s.push_str(&format!("- {}\n", humanize_speakers_in_text(kp)));
        }
        s.push('\n');
    }
    if !sec.action_items.is_empty() {
        s.push_str("**Action items:**\n");
        for a in &sec.action_items {
            // Humanize owner if it's a speaker_N reference, otherwise use verbatim
            let owner = a
                .owner
                .as_ref()
                .map(|o| humanize_speaker_id(o))
                .unwrap_or_else(|| "owner TBD".to_string());
            s.push_str(&format!(
                "- {} (owner: {owner})\n",
                humanize_speakers_in_text(&a.description)
            ));
        }
        s.push('\n');
    }
    if !sec.screenshots.is_empty() {
        s.push_str(&render_screenshots(
            &sec.screenshots,
            sec.index,
            dialect,
            screenshots_dir,
            assets,
        )?);
    }
    Ok(s)
}

fn render_screenshots(
    shots: &[Screenshot],
    section_index: u32,
    dialect: MarkdownDialect,
    screenshots_dir: &Path,
    assets: &mut Vec<PathBuf>,
) -> Result<String, ExportError> {
    if !screenshots_dir.exists() {
        fs::create_dir_all(screenshots_dir)?;
    }
    let mut s = String::new();
    let imgs: Vec<String> = shots
        .iter()
        .map(|shot| -> Result<String, ExportError> {
            let original = shot
                .path
                .file_name()
                .ok_or_else(|| ExportError::Render("screenshot has no file_name".into()))?;
            let ext = shot
                .path
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            // Stable, collision-free name: section + timestamp.
            let unique_name = format!("section{section_index:02}_{:08}{ext}", shot.ms_since_start);
            let dest = screenshots_dir.join(&unique_name);
            fs::copy(&shot.path, &dest)?;
            assets.push(dest.clone());
            let alt = shot
                .caption
                .clone()
                .unwrap_or_else(|| original.to_string_lossy().to_string());
            Ok(format!("![{alt}](screenshots/{unique_name})"))
        })
        .collect::<Result<_, _>>()?;
    match dialect {
        MarkdownDialect::NotionEnhanced => {
            s.push_str("\n<table>\n");
            for chunk in imgs.chunks(2) {
                s.push_str("  <tr>");
                for img in chunk {
                    s.push_str(&format!("<td>{img}</td>"));
                }
                s.push_str("</tr>\n");
            }
            s.push_str("</table>\n\n");
        }
        MarkdownDialect::Basic => {
            for img in imgs {
                s.push_str(&img);
                s.push_str("\n\n");
            }
        }
    }
    Ok(s)
}

fn format_mmss(ms: u64) -> String {
    let total_s = ms / 1000;
    let m = total_s / 60;
    let s = total_s % 60;
    format!("{m}:{s:02}")
}

fn yaml_scalar(s: &str) -> String {
    // Quote-by-default: emit a plain (unquoted) scalar ONLY when the value is
    // unambiguously safe. Everything else is double-quoted with escaping, which
    // a YAML parser always reads back as the exact source string. This closes
    // the YAML 1.1 number-format edge cases (1_000, .inf, 0xDE_AD, ...) in one
    // rule instead of enumerating every shape that must be quoted.
    if is_safe_plain_scalar(s) {
        return s.to_string();
    }
    let mut escaped = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            // Any remaining C0 control char (U+0000–U+001F) is invalid in a
            // YAML double-quoted scalar unless escaped; emit `\xNN` with two
            // lowercase hex digits (e.g. 0x01 -> \x01, 0x07 -> \x07).
            c if (c as u32) < 0x20 => escaped.push_str(&format!("\\x{:02x}", c as u32)),
            c => escaped.push(c),
        }
    }
    format!("\"{escaped}\"")
}

/// A value is safe to emit as a plain scalar only when it matches the strict
/// pattern `^[A-Za-z][A-Za-z0-9 ._/-]*$`, is not a YAML reserved word, and is
/// not number-like. The leading-letter requirement already excludes most
/// numbers; the reserved-word and number checks catch the letter-leading edges
/// (`true`, `inf`, `nan`, ...) that would otherwise slip through unquoted.
fn is_safe_plain_scalar(s: &str) -> bool {
    matches_plain_pattern(s) && !is_yaml_reserved(s) && !looks_like_number(s)
}

/// Matches `^[A-Za-z][A-Za-z0-9 ._/-]*$`: first char is an ASCII letter, the
/// rest are ASCII alphanumerics or one of space, `.`, `_`, `/`, `-`.
fn matches_plain_pattern(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '/' | '-'))
}

/// YAML reserved words (case-insensitive) that a parser would coerce to a
/// boolean, null, or float rather than a string.
fn is_yaml_reserved(s: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "true", "false", "null", "yes", "no", "on", "off", "y", "n", "~", ".inf", "-.inf", ".nan",
    ];
    let lower = s.to_lowercase();
    KEYWORDS.contains(&lower.as_str())
}

/// Returns true if the value could be read back as a YAML number. Three
/// detection paths, checked in order:
///
/// 1. **Parse** — `i64::parse` or `f64::parse` succeeds (decimal integers,
///    floats, scientific notation, a leading or trailing dot, optional sign).
/// 2. **Radix prefix** — an optionally-signed `0x` / `0o` / `0b` prefix with
///    digits valid for that base. Rust's `parse` rejects these, so they are
///    matched explicitly.
/// 3. **Underscore grouping** — YAML 1.1 digit separators (`1_000`,
///    `1_000.5`), which Rust's `parse` also rejects.
///
/// `f64::parse` accepts the bare tokens `inf` / `nan`, so those are flagged
/// here. The dotted YAML spellings `.inf` / `-.inf` / `.nan` are instead caught
/// by [`is_yaml_reserved`]. That overlap is intentional: between them the two
/// checks cover every infinity / NaN spelling, and either one alone is enough
/// to force quoting — neither needs to depend on the other's exact coverage.
fn looks_like_number(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.parse::<i64>().is_ok() || trimmed.parse::<f64>().is_ok() {
        return true;
    }
    // Radix prefixes, optionally signed (Rust's `parse` rejects these).
    let body = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    let lower = body.to_ascii_lowercase();
    for (prefix, radix) in [("0x", 16u32), ("0o", 8), ("0b", 2)] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return !rest.is_empty() && rest.chars().all(|c| c == '_' || c.is_digit(radix));
        }
    }
    // Underscore digit grouping (YAML 1.1: `1_000`, `1_000.5`). Rust's `parse`
    // rejects underscores, so detect them explicitly.
    if trimmed.contains('_') {
        let cleaned = body.replace('_', "");
        let dot_count = cleaned.matches('.').count();
        let digits_only = cleaned.replace('.', "");
        return dot_count <= 1
            && !digits_only.is_empty()
            && digits_only.chars().all(|c| c.is_ascii_digit());
    }
    false
}

fn yaml_list(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let quoted: Vec<String> = items.iter().map(|s| yaml_scalar(s)).collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::{humanize_speaker_id, humanize_speakers_in_text, yaml_scalar};

    #[test]
    fn plain_string_is_unquoted() {
        assert_eq!(yaml_scalar("hello"), "hello");
    }

    #[test]
    fn empty_string_is_double_quoted() {
        assert_eq!(yaml_scalar(""), "\"\"");
    }

    #[test]
    fn single_quote_triggers_double_quoting() {
        assert_eq!(yaml_scalar("it's-great"), "\"it's-great\"");
        assert_eq!(yaml_scalar("O'Reilly"), "\"O'Reilly\"");
    }

    #[test]
    fn double_quote_is_escaped_inside_double_quotes() {
        assert_eq!(yaml_scalar("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn newline_is_escaped_inside_double_quotes() {
        assert_eq!(yaml_scalar("line1\nline2"), "\"line1\\nline2\"");
    }

    #[test]
    fn hash_triggers_double_quoting() {
        assert_eq!(yaml_scalar("foo#bar"), "\"foo#bar\"");
    }

    #[test]
    fn colon_space_triggers_double_quoting() {
        assert_eq!(yaml_scalar("key: value"), "\"key: value\"");
    }

    #[test]
    fn leading_special_char_triggers_double_quoting() {
        assert_eq!(yaml_scalar(":starts-with-colon"), "\":starts-with-colon\"");
        assert_eq!(yaml_scalar("-starts-with-dash"), "\"-starts-with-dash\"");
        assert_eq!(
            yaml_scalar("%starts-with-percent"),
            "\"%starts-with-percent\""
        );
    }

    #[test]
    fn trailing_colon_triggers_double_quoting() {
        assert_eq!(yaml_scalar("key:"), "\"key:\"");
    }

    #[test]
    fn whitespace_only_string_is_double_quoted() {
        assert_eq!(yaml_scalar("   "), "\"   \"");
        assert_eq!(yaml_scalar("\t"), "\"\\t\"");
    }

    #[test]
    fn tab_is_escaped_inside_double_quotes() {
        assert_eq!(yaml_scalar("col1\tcol2"), "\"col1\\tcol2\"");
    }

    #[test]
    fn control_chars_are_hex_escaped_inside_double_quotes() {
        // U+0001 (SOH) and U+0007 (BEL) have no dedicated escape and would be
        // invalid raw inside a YAML double-quoted scalar; they must emit as
        // \x01 / \x07 (two lowercase hex digits).
        let mut value = String::from("a");
        value.push('\u{0001}');
        value.push('\u{0007}');
        value.push('b');
        assert_eq!(yaml_scalar(&value), "\"a\\x01\\x07b\"");
    }

    #[test]
    fn yaml_boolean_keywords_are_double_quoted() {
        assert_eq!(yaml_scalar("true"), "\"true\"");
        assert_eq!(yaml_scalar("false"), "\"false\"");
        assert_eq!(yaml_scalar("True"), "\"True\"");
        assert_eq!(yaml_scalar("FALSE"), "\"FALSE\"");
    }

    #[test]
    fn yaml_null_keywords_are_double_quoted() {
        assert_eq!(yaml_scalar("null"), "\"null\"");
        assert_eq!(yaml_scalar("Null"), "\"Null\"");
        assert_eq!(yaml_scalar("~"), "\"~\"");
    }

    #[test]
    fn yaml_yes_no_keywords_are_double_quoted() {
        assert_eq!(yaml_scalar("yes"), "\"yes\"");
        assert_eq!(yaml_scalar("no"), "\"no\"");
        assert_eq!(yaml_scalar("on"), "\"on\"");
        assert_eq!(yaml_scalar("off"), "\"off\"");
        assert_eq!(yaml_scalar("y"), "\"y\"");
        assert_eq!(yaml_scalar("n"), "\"n\"");
    }

    #[test]
    fn numbers_are_double_quoted() {
        // Integers
        assert_eq!(yaml_scalar("0"), "\"0\"");
        assert_eq!(yaml_scalar("123"), "\"123\"");
        assert_eq!(yaml_scalar("-42"), "\"-42\"");
        assert_eq!(yaml_scalar("+7"), "\"+7\"");
        // Floats
        assert_eq!(yaml_scalar("3.14"), "\"3.14\"");
        assert_eq!(yaml_scalar(".5"), "\".5\"");
        assert_eq!(yaml_scalar("5."), "\"5.\"");
        assert_eq!(yaml_scalar("-0.5"), "\"-0.5\"");
        // Hex
        assert_eq!(yaml_scalar("0x1A"), "\"0x1A\"");
        // Octal
        assert_eq!(yaml_scalar("0o755"), "\"0o755\"");
        // Binary
        assert_eq!(yaml_scalar("0b1010"), "\"0b1010\"");
        // Scientific notation
        assert_eq!(yaml_scalar("1e5"), "\"1e5\"");
        assert_eq!(yaml_scalar("1.5e-3"), "\"1.5e-3\"");
    }

    #[test]
    fn number_shaped_strings_are_double_quoted() {
        // Leading digits or dots fail the safe-plain pattern, so quote-by-default
        // wraps them even though they aren't valid numbers on their own.
        assert_eq!(yaml_scalar("0x"), "\"0x\""); // incomplete hex
        assert_eq!(yaml_scalar("1.2.3"), "\"1.2.3\""); // multiple dots
    }

    #[test]
    fn letter_leading_strings_stay_unquoted() {
        // These match the safe-plain pattern and aren't number-like, so they
        // remain plain scalars.
        assert_eq!(yaml_scalar("abc123"), "abc123"); // letters before digits
        assert_eq!(yaml_scalar("v1.0"), "v1.0"); // letter prefix
    }

    #[test]
    fn yaml_1_1_number_formats_are_double_quoted() {
        assert_eq!(yaml_scalar("1_000"), "\"1_000\""); // underscore grouping
        assert_eq!(yaml_scalar(".inf"), "\".inf\""); // YAML infinity
        assert_eq!(yaml_scalar(".nan"), "\".nan\""); // YAML not-a-number
        assert_eq!(yaml_scalar("0xDEAD"), "\"0xDEAD\""); // unsigned hex
    }

    #[test]
    fn title_with_colon_is_double_quoted() {
        assert_eq!(yaml_scalar("Q3: planning"), "\"Q3: planning\"");
    }

    #[test]
    fn leading_zero_string_is_double_quoted() {
        // A string like "007" must round-trip as a string, not octal/decimal.
        assert_eq!(yaml_scalar("007"), "\"007\"");
    }

    #[test]
    fn humanize_speaker_id_converts_zero_to_one() {
        assert_eq!(humanize_speaker_id("speaker_0"), "Speaker 1");
    }

    #[test]
    fn humanize_speaker_id_converts_one_to_two() {
        assert_eq!(humanize_speaker_id("speaker_1"), "Speaker 2");
    }

    #[test]
    fn humanize_speaker_id_returns_original_for_non_matching() {
        assert_eq!(humanize_speaker_id("Alice"), "Alice");
        assert_eq!(humanize_speaker_id("speaker"), "speaker");
        assert_eq!(humanize_speaker_id("speaker_"), "speaker_");
    }

    #[test]
    fn humanize_speakers_in_text_replaces_multiple_refs() {
        assert_eq!(
            humanize_speakers_in_text("speaker_0 said to speaker_1"),
            "Speaker 1 said to Speaker 2"
        );
    }

    #[test]
    fn humanize_speakers_in_text_handles_markdown_format() {
        assert_eq!(
            humanize_speakers_in_text("**speaker_0:** hello"),
            "**Speaker 1:** hello"
        );
    }

    #[test]
    fn humanize_speakers_in_text_preserves_surrounding_text() {
        assert_eq!(
            humanize_speakers_in_text("The discussion between speaker_0 and speaker_2 continued"),
            "The discussion between Speaker 1 and Speaker 3 continued"
        );
    }
}
