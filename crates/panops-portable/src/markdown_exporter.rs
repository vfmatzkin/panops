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

fn render_frontmatter(notes: &StructuredNotes) -> String {
    let fm = &notes.frontmatter;
    let mut s = String::from("---\n");
    s.push_str(&format!("title: {}\n", yaml_scalar(&fm.title)));
    s.push_str(&format!("date: {}\n", fm.date.format("%Y-%m-%d")));
    s.push_str(&format!("started_at: {}\n", fm.started_at.to_rfc3339()));
    s.push_str(&format!("duration_ms: {}\n", fm.duration_ms));
    s.push_str(&format!("language: {}\n", yaml_scalar(&notes.language)));
    s.push_str("speakers:\n");
    for sp in &fm.speakers {
        s.push_str(&format!("  - {}\n", yaml_scalar(sp)));
    }
    s.push_str("tags:\n");
    for tag in &fm.tags {
        s.push_str(&format!("  - {}\n", yaml_scalar(tag)));
    }
    s.push_str(&format!("template: {}\n", yaml_scalar(&fm.template)));
    s.push_str(&format!(
        "dialect: {}\n",
        match fm.dialect {
            MarkdownDialect::NotionEnhanced => "notion-enhanced",
            MarkdownDialect::Basic => "basic",
        }
    ));
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
    s.push_str(sec.narrative_md.trim());
    s.push_str("\n\n");
    if !sec.key_points.is_empty() {
        s.push_str("**Key points:**\n");
        for kp in &sec.key_points {
            s.push_str(&format!("- {kp}\n"));
        }
        s.push('\n');
    }
    if !sec.action_items.is_empty() {
        s.push_str("**Action items:**\n");
        for a in &sec.action_items {
            let owner = a.owner.as_deref().unwrap_or("owner TBD");
            s.push_str(&format!("- {} (owner: {owner})\n", a.description));
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
        || s.contains(['\n', '\r', '"', '\\', '#'])
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
