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
    // YAML reserved words that would be parsed as booleans/null.
    const YAML_KEYWORDS: &[&str] = &[
        "true", "false", "null", "yes", "no", "on", "off", "y", "n", "~",
    ];
    // Characters that trigger quoting anywhere in the string.
    const QUOTE_CHARS: &[char] = &['\n', '\r', '\t', '"', '\\', '#', '\''];
    // Characters that trigger quoting if the string starts with them.
    const LEADING_SPECIAL: &[char] = &[
        ':', '-', '!', '|', '>', '[', ']', '{', '}', '*', '&', '?', '@', '`', '%',
    ];

    let lower = s.to_lowercase();
    let needs_quoting = s.is_empty()
        // Whitespace-only strings need quoting
        || s.trim().is_empty()
        // YAML keywords (booleans/null)
        || YAML_KEYWORDS.contains(&lower.as_str())
        // Strings that look like numbers (integers, floats, hex, octal)
        || looks_like_number(s)
        // Colon-space (mapping indicator) or trailing colon (key indicator)
        || s.contains(": ") || s.ends_with(':')
        // Special characters anywhere
        || s.contains(QUOTE_CHARS)
        // Special characters at start
        || s.starts_with(LEADING_SPECIAL);

    if needs_quoting {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

/// Returns true if the string would be parsed as a YAML number.
fn looks_like_number(s: &str) -> bool {
    // YAML parses integers, floats, hex, octal, and binary as numbers.
    // We conservatively quote anything that might be parsed as a number.
    if s.is_empty() {
        return false;
    }
    // Hex: 0x...
    if s.starts_with("0x") || s.starts_with("0X") {
        return s.len() > 2 && s[2..].chars().all(|c| c.is_ascii_hexdigit());
    }
    // Octal: 0o... (YAML 1.2 uses 0o prefix)
    if s.starts_with("0o") || s.starts_with("0O") {
        return s.len() > 2 && s[2..].chars().all(|c| c.is_ascii_digit() && c < '8');
    }
    // Binary: 0b...
    if s.starts_with("0b") || s.starts_with("0B") {
        return s.len() > 2 && s[2..].chars().all(|c| c == '0' || c == '1');
    }

    // Check for plain integers and floats.
    // A valid YAML float has exactly ONE dot.
    let trimmed = s.trim();

    // Count dots - more than one means it's not a valid number
    let dot_count = trimmed.chars().filter(|c| *c == '.').count();
    if dot_count > 1 {
        return false;
    }

    // Handle optional sign prefix
    let digits_part = if trimmed.starts_with('+') || trimmed.starts_with('-') {
        &trimmed[1..]
    } else {
        trimmed
    };

    // Integer: all digits, no dot
    if dot_count == 0 && digits_part.chars().all(|c| c.is_ascii_digit()) && !digits_part.is_empty()
    {
        return true;
    }

    // Float: exactly one dot, rest are digits (allow ".5" and "5.")
    if dot_count == 1 {
        let without_dot = digits_part.replace('.', "");
        // Empty after removing dot means just "." which isn't valid, but
        // ".5" or "5." are valid YAML floats
        if without_dot.chars().all(|c| c.is_ascii_digit()) && !without_dot.is_empty() {
            return true;
        }
    }

    // Scientific notation (e.g., "1e5", "1.5e-3")
    if trimmed.contains('e') || trimmed.contains('E') {
        let parts: Vec<&str> = trimmed.split(['e', 'E']).collect();
        if parts.len() == 2 {
            let base = parts[0];
            let exp = parts[1];
            // Base part can be integer or float (one dot max)
            let base_dot_count = base.chars().filter(|c| *c == '.').count();
            let base_digits = base.replace(['.', '+', '-'], "");
            let base_ok = base_dot_count <= 1
                && base_digits.chars().all(|c| c.is_ascii_digit())
                && !base_digits.is_empty();
            // Exp part is integer (may have sign)
            let exp_digits = exp.replace(['+', '-'], "");
            let exp_ok = exp_digits.chars().all(|c| c.is_ascii_digit()) && !exp_digits.is_empty();
            if base_ok && exp_ok {
                return true;
            }
        }
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
    fn non_number_strings_are_unquoted() {
        // Strings that look like numbers but aren't valid
        assert_eq!(yaml_scalar("0x"), "0x"); // incomplete hex
        assert_eq!(yaml_scalar("1.2.3"), "1.2.3"); // multiple dots
        assert_eq!(yaml_scalar("abc123"), "abc123"); // letters before digits
        assert_eq!(yaml_scalar("v1.0"), "v1.0"); // letter prefix
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
