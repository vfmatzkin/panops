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
            return format!("Speaker {}", id + 1);
        }
    }
    raw.to_string()
}

/// Replace all `speaker_N` patterns in text with `Speaker N+1`.
/// Used for narrative content where speaker references may appear.
fn humanize_speakers_in_text(text: &str) -> String {
    // Replace all `speaker_N` patterns. The pattern appears in markdown as:
    // - `speaker_0` in prose
    // - `**speaker_0:**` in fallback transcript dumps
    // Use a simple regex-like replacement since we control the format.
    let mut result = text.to_string();
    // Find and replace each speaker_N pattern
    let mut i = 0;
    while i < result.len() {
        if result[i..].starts_with("speaker_") {
            // Find the end of the number
            let num_start = i + 8;
            let num_end = result[num_start..]
                .find(|c: char| !c.is_ascii_digit())
                .map(|pos| num_start + pos)
                .unwrap_or(result.len());
            if num_start < num_end {
                if let Ok(id) = result[num_start..num_end].parse::<u32>() {
                    let replacement = format!("Speaker {}", id + 1);
                    result.replace_range(i..num_end, &replacement);
                    i += replacement.len();
                    continue;
                }
            }
        }
        i += 1;
    }
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
    let needs_quoting = s.is_empty()
        || s.contains(['\n', '\r', '"', '\\', '#', '\''])
        || s.contains(": ")
        || s.starts_with([
            ':', '-', '!', '|', '>', '[', ']', '{', '}', '*', '&', '?', '@', '`',
        ]);
    if needs_quoting {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
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
