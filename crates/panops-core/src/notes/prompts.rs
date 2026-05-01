//! Prompt builders for the notes pipeline's LLM stages.

use std::fmt::Write as _;

use crate::Segment;
use crate::llm::LlmRequest;

use super::dialect::MarkdownDialect;

pub const SECTION_NARRATIVE_TEMPERATURE: f32 = 0.6;
pub const FRONTMATTER_TEMPERATURE: f32 = 0.3;
pub const SECTION_NARRATIVE_MAX_TOKENS: u32 = 2048;
pub const FRONTMATTER_MAX_TOKENS: u32 = 512;

/// Compact summary of a section, fed to the frontmatter prompt.
#[derive(Debug, Clone)]
pub struct SectionSummary {
    pub title: String,
    pub key_points: Vec<String>,
}

const SECTION_NARRATIVE_SYSTEM: &str = "\
You are an expert meeting-notes writer. You receive a diarized transcript
section and produce a structured narrative summary. You write in clear,
neutral prose. You NEVER attribute a quote or statement to a specific speaker
unless that segment carries a confirmed speaker_id from diarization. When
attribution is ambiguous, write in passive voice (\"the team discussed\", \"a
concern was raised\", \"it was proposed that\"). You return a single JSON object
with the schema you are given.";

const FRONTMATTER_SYSTEM: &str = "\
You are an expert meeting-notes editor. You receive a list of section titles
and key points and produce a meeting title and tag list. Title is concise
(<=80 chars), descriptive, neutral. Tags are lowercase kebab-case, max 10,
factual (no marketing). You return a single JSON object with the schema you
are given.";

pub fn build_section_narrative_prompt(
    segments: &[Segment],
    dialect: MarkdownDialect,
    language: &str,
) -> LlmRequest {
    let transcript = render_transcript(segments);
    let cheat = dialect.cheat_sheet();
    let user = format!(
        "Section transcript (diarized; speaker_X is a stable label per voice):\n\n\
         {transcript}\n\
         Markdown dialect for `narrative_md`:\n\
         {cheat}\n\
         Output language: {language}\n\n\
         Speaker attribution rule (STRICT): never attribute a quote to a speaker_id\n\
         that does not appear in the transcript. When in doubt, use passive voice.\n\n\
         Return JSON matching exactly:\n\
         {{\n  \"title\": \"string (descriptive, <80 chars)\",\n  \"narrative_md\": \"string (markdown body in the dialect above; 100–600 words)\",\n  \"key_points\": [\"string\", ...] (0–6 short bullets),\n  \"action_items\": [{{\"description\": \"string\", \"owner\": \"speaker_0\"}}] or [{{\"description\": \"string\", \"owner\": null}}]\n}}"
    );
    LlmRequest {
        system: Some(SECTION_NARRATIVE_SYSTEM.to_string()),
        user,
        schema: Some(section_narrative_schema()),
        temperature: SECTION_NARRATIVE_TEMPERATURE,
        max_tokens: SECTION_NARRATIVE_MAX_TOKENS,
    }
}

pub fn build_frontmatter_prompt(
    summaries: &[SectionSummary],
    language: &str,
    duration_ms: u64,
) -> LlmRequest {
    let mut s = String::new();
    for (i, sum) in summaries.iter().enumerate() {
        let _ = writeln!(s, "Section {}: {}", i + 1, sum.title);
        for kp in &sum.key_points {
            let _ = writeln!(s, "  - {kp}");
        }
    }
    let user = format!(
        "Section summaries:\n\n\
         {s}\n\
         Meeting language: {language}\n\
         Meeting duration: {duration_ms} ms\n\n\
         Return JSON matching exactly:\n\
         {{\n  \"title\": \"string (<=80 chars, descriptive)\",\n  \"tags\": [\"lowercase-kebab-case\", ...] (max 10)\n}}"
    );
    LlmRequest {
        system: Some(FRONTMATTER_SYSTEM.to_string()),
        user,
        schema: Some(frontmatter_schema()),
        temperature: FRONTMATTER_TEMPERATURE,
        max_tokens: FRONTMATTER_MAX_TOKENS,
    }
}

fn render_transcript(segments: &[Segment]) -> String {
    let mut out = String::new();
    for seg in segments {
        let label = match seg.speaker_id {
            Some(id) => format!("speaker_{id}"),
            None => "unknown".to_string(),
        };
        let start = seg.start_ms / 1000;
        let end = seg.end_ms / 1000;
        out.push_str(&format!("[{start:>4}–{end:>4}s] {label}: {}\n", seg.text));
    }
    out
}

fn section_narrative_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["title", "narrative_md", "key_points", "action_items"],
        "properties": {
            "title": {"type": "string"},
            "narrative_md": {"type": "string"},
            "key_points": {"type": "array", "items": {"type": "string"}},
            "action_items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["description"],
                    "properties": {
                        "description": {"type": "string"},
                        "owner": {"type": ["string", "null"]}
                    }
                }
            }
        }
    })
}

fn frontmatter_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["title", "tags"],
        "properties": {
            "title": {"type": "string"},
            "tags": {"type": "array", "items": {"type": "string"}}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Segment;
    use crate::notes::dialect::MarkdownDialect;

    fn seg(start: u64, end: u64, speaker: Option<u32>, text: &str) -> Segment {
        Segment {
            start_ms: start,
            end_ms: end,
            text: text.into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            is_partial: false,
            speaker_id: speaker,
        }
    }

    #[test]
    fn section_narrative_prompt_includes_speaker_attribution_rule() {
        let segs = vec![seg(0, 5000, Some(0), "hello")];
        let p = build_section_narrative_prompt(&segs, MarkdownDialect::NotionEnhanced, "en");
        assert!(p.user.contains("speaker_id"));
        assert!(p.user.contains("passive voice"));
    }

    #[test]
    fn section_narrative_prompt_includes_dialect_cheat_sheet() {
        let segs = vec![seg(0, 5000, Some(0), "hello")];
        let p = build_section_narrative_prompt(&segs, MarkdownDialect::NotionEnhanced, "en");
        assert!(p.user.contains("<callout"));
        let p = build_section_narrative_prompt(&segs, MarkdownDialect::Basic, "en");
        assert!(!p.user.contains("<callout"));
    }

    #[test]
    fn section_narrative_prompt_renders_each_segment_with_speaker_id() {
        let segs = vec![
            seg(0, 5000, Some(0), "hello"),
            seg(5000, 10000, Some(1), "hi"),
        ];
        let p = build_section_narrative_prompt(&segs, MarkdownDialect::Basic, "en");
        assert!(p.user.contains("speaker_0"));
        assert!(p.user.contains("speaker_1"));
        assert!(p.user.contains("hello"));
        assert!(p.user.contains("hi"));
    }

    #[test]
    fn frontmatter_prompt_includes_section_titles_and_key_points() {
        let summaries = vec![
            SectionSummary {
                title: "Intro".into(),
                key_points: vec!["one".into(), "two".into()],
            },
            SectionSummary {
                title: "Wrap".into(),
                key_points: vec![],
            },
        ];
        let p = build_frontmatter_prompt(&summaries, "en", 60_000);
        assert!(p.user.contains("Intro"));
        assert!(p.user.contains("Wrap"));
        assert!(p.user.contains("one"));
    }
}
