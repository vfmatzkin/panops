//! Prompt builders for the notes pipeline's LLM stages.

use std::fmt::Write as _;

use crate::Segment;
use crate::llm::LlmRequest;

use super::dialect::MarkdownDialect;

pub const SECTION_NARRATIVE_TEMPERATURE: f32 = 0.6;
pub const FRONTMATTER_TEMPERATURE: f32 = 0.3;
pub const SECTION_NARRATIVE_MAX_TOKENS: u32 = 2048;
pub const FRONTMATTER_MAX_TOKENS: u32 = 512;

/// Approximate context-window threshold for section chunking.
/// gemma3:4b has ~4k token context; we reserve headroom for system prompt,
/// schema, and output tokens. Using chars/4 heuristic (~4 chars/token for
/// English), 8000 chars ≈ 2000 input tokens, leaving ~2k for output.
/// Sections exceeding this threshold are split into ordered sub-chunks.
pub const SECTION_CHUNK_THRESHOLD_CHARS: usize = 8000;

/// Target size for each sub-chunk when splitting a long section.
/// Chunks are built to this target but may vary slightly due to
/// segment-boundary constraints. Half the threshold allows room for
/// the merge prompt's overhead when combining sub-summaries.
pub const SECTION_CHUNK_TARGET_CHARS: usize = 4000;

/// Approximate rendered transcript overhead per segment line.
/// Line format: "[XXXX–YYYYs] speaker_N: TEXT\n".
const SEGMENT_LINE_OVERHEAD: usize = 20;

/// Minimum silence gap treated as a semantic chunk boundary.
const SPLIT_GAP_THRESHOLD_MS: u64 = 2000;

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
         Non-duplication rule (STRICT): `narrative_md`, `key_points`, and `action_items`\n\
         must NOT restate the same facts in different shapes. They are three\n\
         distinct views of the section:\n\
         - `narrative_md`: connective prose — context, who arrived/left, how the\n\
           conversation moved, transitions between topics, mood/tone where it\n\
           matters. NO bullet lists. NO embedded \"Key points:\" or \"Action items:\"\n\
           sections. Do NOT restate the bullets in prose form.\n\
         - `key_points`: durable takeaways the reader scans for later — facts,\n\
           numbers, decisions, named outcomes. Punchy and self-contained. Each\n\
           bullet is a fact that does NOT appear (in any paraphrase) in\n\
           `narrative_md`.\n\
         - `action_items`: explicit commitments — \"who will do what by when\".\n\
           A commitment that is also a key takeaway belongs HERE, not in\n\
           `key_points`. Owner is a speaker_id (or null when unassigned).\n\n\
         Return JSON matching exactly:\n\
         {{\n  \"title\": \"string (descriptive, <80 chars)\",\n  \"narrative_md\": \"string (prose only — no bullets — in the dialect above; up to 400 words, scaled to section length)\",\n  \"key_points\": [\"string\", ...] (0–6 short bullets, each a fact NOT in narrative_md),\n  \"action_items\": [{{\"description\": \"string\", \"owner\": \"speaker_0\"}}] or [{{\"description\": \"string\", \"owner\": null}}]\n}}"
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

/// Estimate the character count of a rendered transcript for a slice of segments.
/// Used to determine if chunking is needed and to build chunks of target size.
pub fn estimate_transcript_chars(segments: &[Segment]) -> usize {
    // Each segment line format: "[XXXX–YYYYs] speaker_N: TEXT\n".
    segments
        .iter()
        .map(|s| s.text.len() + SEGMENT_LINE_OVERHEAD)
        .sum()
}

/// Estimate the input size contribution of one chunk-summary JSON value.
///
/// This intentionally uses serialized JSON length as a cheap, conservative-ish
/// proxy for the text `build_merge_section_prompt` will render. The rendered
/// prompt adds labels/instructions around these fields, so callers should still
/// reserve prompt headroom or verify the final prompt size before sending it.
pub fn estimate_chunk_summary_chars(summary: &serde_json::Value) -> usize {
    serde_json::to_string(summary).map_or(0, |s| s.len())
}

/// Split segments into ordered sub-chunks, each respecting segment boundaries.
/// Returns non-empty chunks where each chunk's rendered transcript is close to
/// `target_chars` but never exceeds `max_chars`. The last chunk may be smaller.
/// Guarantees: every segment appears in exactly one chunk, order preserved.
pub fn split_segments_for_chunking(
    segments: &[Segment],
    target_chars: usize,
    max_chars: usize,
) -> Vec<Vec<Segment>> {
    if segments.is_empty() {
        return Vec::new();
    }

    let total_chars = estimate_transcript_chars(segments);
    if total_chars <= max_chars {
        // No chunking needed
        return vec![segments.to_vec()];
    }

    let mut chunks: Vec<Vec<Segment>> = Vec::new();
    let mut current: Vec<Segment> = Vec::new();
    let mut current_chars = 0;

    for seg in segments {
        let seg_chars = seg.text.len() + SEGMENT_LINE_OVERHEAD;

        // If adding this segment would exceed max_chars AND we already have content,
        // flush the current chunk and start fresh.
        if current_chars + seg_chars > max_chars && !current.is_empty() {
            chunks.push(current);
            current = Vec::new();
            current_chars = 0;
        }

        // If we're over target and this is a natural boundary (gap or speaker change),
        // consider flushing. This helps keep chunks semantically coherent.
        if current_chars >= target_chars && !current.is_empty() {
            let prev = current.last().unwrap();
            let gap = seg.start_ms.saturating_sub(prev.end_ms);
            let speaker_change = seg.speaker_id != prev.speaker_id;
            if gap > SPLIT_GAP_THRESHOLD_MS || speaker_change {
                chunks.push(current);
                current = Vec::new();
                current_chars = 0;
            }
        }

        current.push(seg.clone());
        current_chars += seg_chars;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    // Ensure no chunk exceeds max_chars (shouldn't happen with logic above,
    // but verify for safety). If a single segment exceeds max_chars, it stays
    // alone — we can't split mid-segment.
    debug_assert!(
        chunks
            .iter()
            .all(|c| estimate_transcript_chars(c) <= max_chars || c.len() == 1),
        "chunk exceeds max_chars"
    );

    chunks
}

/// Prompt for merging multiple sub-chunk summaries into a final section summary.
/// Takes JSON outputs from chunk-level calls and produces a unified result.
pub fn build_merge_section_prompt(
    chunk_summaries: &[serde_json::Value],
    dialect: MarkdownDialect,
    language: &str,
) -> LlmRequest {
    let mut summaries_text = String::new();
    for (i, summary) in chunk_summaries.iter().enumerate() {
        let title = summary
            .get("title")
            .and_then(|s| s.as_str())
            .unwrap_or("Untitled chunk");
        let narrative = summary
            .get("narrative_md")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let key_points = summary
            .get("key_points")
            .and_then(|s| s.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();
        let action_items = summary
            .get("action_items")
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| {
                        let o = x.as_object()?;
                        let desc = o.get("description")?.as_str()?;
                        let owner = o.get("owner").and_then(|v| v.as_str());
                        Some(format!(
                            "- {} (owner: {})",
                            desc,
                            owner.unwrap_or("unassigned")
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let _ = writeln!(summaries_text, "Chunk {}:", i + 1);
        let _ = writeln!(summaries_text, "  Title: {}", title);
        let _ = writeln!(summaries_text, "  Narrative: {}", narrative);
        for kp in &key_points {
            let _ = writeln!(summaries_text, "  Key point: {}", kp);
        }
        for ai in &action_items {
            let _ = writeln!(summaries_text, "  Action: {}", ai);
        }
        let _ = writeln!(summaries_text);
    }

    let cheat = dialect.cheat_sheet();
    let user = format!(
        "Sub-chunk summaries (ordered chronologically):\n\n\
         {summaries_text}\n\
         Markdown dialect for `narrative_md`:\n\
         {cheat}\n\
         Output language: {language}\n\n\
         Merge these sub-chunks into a single unified section summary.\n\
         - Combine narratives into flowing prose (no bullet lists).\n\
         - Deduplicate key_points: keep distinct facts, merge overlapping.\n\
         - Deduplicate action_items: keep distinct commitments.\n\
         - Title should represent the unified theme.\n\n\
         Return JSON matching exactly:\n\
         {{\n  \"title\": \"string (descriptive, <80 chars)\",\n  \"narrative_md\": \"string (prose only, in the dialect above)\",\n  \"key_points\": [\"string\", ...],\n  \"action_items\": [{{\"description\": \"string\", \"owner\": \"speaker_0\"}}] or [{{\"description\": \"string\", \"owner\": null}}]\n}}"
    );

    LlmRequest {
        system: Some(SECTION_NARRATIVE_SYSTEM.to_string()),
        user,
        schema: Some(section_narrative_schema()),
        temperature: SECTION_NARRATIVE_TEMPERATURE,
        max_tokens: SECTION_NARRATIVE_MAX_TOKENS,
    }
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
    fn section_narrative_prompt_includes_non_duplication_rule() {
        let segs = vec![seg(0, 5000, Some(0), "hello")];
        let p = build_section_narrative_prompt(&segs, MarkdownDialect::Basic, "en");
        assert!(p.user.contains("Non-duplication rule"));
        assert!(p.user.contains("must NOT restate the same facts"));
        assert!(p.user.contains("NO bullet lists"));
        assert!(p.user.contains("NOT in narrative_md"));
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

    #[test]
    fn estimate_transcript_chars_counts_text_plus_overhead() {
        // Each segment: ~20 chars overhead + text length
        let segs = vec![
            seg(0, 5000, Some(0), "hello"),     // 5 chars text
            seg(5000, 10000, Some(1), "world"), // 5 chars text
        ];
        let estimate = estimate_transcript_chars(&segs);
        assert_eq!(estimate, 50); // 2 * (20 + 5)
    }

    #[test]
    fn split_segments_returns_single_chunk_when_below_threshold() {
        let segs = vec![seg(0, 5000, Some(0), "short")];
        let chunks = split_segments_for_chunking(&segs, 100, 200);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 1);
    }

    #[test]
    fn split_segments_splits_at_segment_boundaries() {
        // Create segments totaling > max_chars with clear boundaries
        let segs: Vec<Segment> = (0..10u32)
            .map(|i| {
                seg(
                    u64::from(i) * 5000,
                    u64::from(i + 1) * 5000,
                    Some(i % 2),
                    "word word word word",
                )
            })
            .collect();
        // Each segment: 20 + 19 = 39 chars, total = 390 chars
        let chunks = split_segments_for_chunking(&segs, 80, 120);
        assert!(chunks.len() > 1, "should split into multiple chunks");
        // Verify no segment is lost
        let total_segments: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total_segments, segs.len());
        // Verify order preserved
        let all_texts: Vec<_> = chunks.iter().flatten().map(|s| s.text.as_str()).collect();
        let expected: Vec<_> = segs.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(all_texts, expected);
    }

    #[test]
    fn split_segments_splits_at_speaker_change_after_target() {
        // Max=50, so total (78 chars) exceeds max. Target=30.
        // After first segment (39 >= target), speaker change should trigger split.
        let segs = vec![
            seg(0, 5000, Some(0), "text text text text"), // 39 chars (20 + 19)
            seg(5000, 10000, Some(1), "text text text text"), // 39 chars, speaker change
        ];
        // Total = 78 chars, max = 50 → chunking required
        let chunks = split_segments_for_chunking(&segs, 30, 50);
        // Should split because after first segment we're >= target and speaker changes
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 1);
        assert_eq!(chunks[1].len(), 1);
    }

    #[test]
    fn split_segments_splits_at_gap_after_target() {
        // Max=50, so total (78 chars) exceeds max. Target=30. Gap of 5s between segments.
        let segs = vec![
            seg(0, 5000, Some(0), "text text text text"), // 39 chars
            seg(10_000, 15_000, Some(0), "text text text text"), // 39 chars, 5s gap
        ];
        // Total = 78 chars, max = 50 → chunking required
        let chunks = split_segments_for_chunking(&segs, 30, 50);
        // Should split because after first segment we're >= target and gap > 2s
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn build_merge_prompt_includes_all_chunk_summaries() {
        let summaries = vec![
            serde_json::json!({"title": "Part 1", "narrative_md": "first part", "key_points": ["a"], "action_items": []}),
            serde_json::json!({"title": "Part 2", "narrative_md": "second part", "key_points": ["b"], "action_items": [{"description": "do it", "owner": null}]}),
        ];
        let p = build_merge_section_prompt(&summaries, MarkdownDialect::Basic, "en");
        assert!(p.user.contains("Part 1"));
        assert!(p.user.contains("Part 2"));
        assert!(p.user.contains("first part"));
        assert!(p.user.contains("second part"));
        assert!(p.user.contains("do it"));
    }
}
