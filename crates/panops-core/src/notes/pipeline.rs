//! `NotesGenerator`: orchestrates the 5 pipeline stages.
//!
//! 1. Topic segmentation (deterministic).
//! 2. Per-section narrative LLM call (parallel).
//! 3. Screenshot anchoring (deterministic).
//! 4. Frontmatter LLM call (single).
//! 5. Render is owned by `NotesExporter`, not this pipeline.
//!
//! A failed per-section LLM call falls back to a transcript-block narrative
//! with a `<!-- panops: llm error -->` marker; the pipeline does not abort.

use std::collections::HashSet;

use chrono::Utc;
use rayon::prelude::*;

use crate::Segment;
use crate::llm::{LlmProvider, LlmResponse};
use crate::notes::dialect::MarkdownDialect;
use crate::notes::error::NotesError;
use crate::notes::input::NotesInput;
use crate::notes::ir::{ActionItem, NotesFrontmatter, NotesSection, StructuredNotes};
use crate::notes::prompts::{
    SectionSummary, build_frontmatter_prompt, build_section_narrative_prompt,
};
use crate::notes::screenshot_anchoring::anchor_screenshots;
use crate::notes::topic_segmentation::{TopicSegmentationConfig, segment_topics};

pub struct NotesGenerator<'a> {
    pub llm: &'a (dyn LlmProvider + 'a),
    pub dialect: MarkdownDialect,
}

impl NotesGenerator<'_> {
    pub fn generate(&self, input: NotesInput) -> Result<StructuredNotes, NotesError> {
        if input.transcript.is_empty() {
            return Err(NotesError::EmptyTranscript);
        }

        let language = input
            .meeting_metadata
            .language_hint
            .clone()
            .unwrap_or_else(|| dominant_language(&input.transcript));

        // Stage 1
        let raw_sections = segment_topics(&input.transcript, &TopicSegmentationConfig::default());

        // Stage 2 (parallel)
        let section_drafts: Vec<SectionDraft> = raw_sections
            .par_iter()
            .map(|raw| {
                let segs: Vec<Segment> = raw
                    .segment_indices
                    .iter()
                    .map(|i| input.transcript[*i].clone())
                    .collect();
                let req = build_section_narrative_prompt(&segs, self.dialect, &language);
                match self.llm.complete(req) {
                    Ok(LlmResponse::Json(v)) => SectionDraft::from_json(raw.time_range_ms, segs, v),
                    Ok(LlmResponse::Text(_)) => SectionDraft::fallback(
                        raw.time_range_ms,
                        segs,
                        "llm returned text, expected json",
                        self.dialect,
                    ),
                    Err(e) => SectionDraft::fallback(
                        raw.time_range_ms,
                        segs,
                        &e.to_string(),
                        self.dialect,
                    ),
                }
            })
            .collect();

        // Stage 3
        let per_section_screenshots = anchor_screenshots(&raw_sections, &input.screenshots);

        // Stage 4 (single LLM call)
        let summaries: Vec<SectionSummary> = section_drafts
            .iter()
            .map(|d| SectionSummary {
                title: d.title.clone(),
                key_points: d.key_points.clone(),
            })
            .collect();
        let fm_req =
            build_frontmatter_prompt(&summaries, &language, input.meeting_metadata.duration_ms);
        let (title, tags) = match self.llm.complete(fm_req)? {
            LlmResponse::Json(v) => extract_frontmatter(v),
            LlmResponse::Text(_) => ("Untitled meeting".to_string(), Vec::new()),
        };

        let speakers = collect_speakers(&input.transcript);

        let sections: Vec<NotesSection> = section_drafts
            .into_iter()
            .zip(per_section_screenshots)
            .enumerate()
            .map(|(i, (d, shots))| NotesSection {
                index: u32::try_from(i + 1).unwrap_or(u32::MAX),
                title: d.title,
                time_range_ms: d.time_range_ms,
                narrative_md: d.narrative_md,
                key_points: d.key_points,
                action_items: d.action_items,
                screenshots: shots,
            })
            .collect();

        Ok(StructuredNotes {
            schema_version: StructuredNotes::SCHEMA_VERSION,
            frontmatter: NotesFrontmatter {
                title,
                date: input.meeting_metadata.started_at.date_naive(),
                started_at: input.meeting_metadata.started_at,
                duration_ms: input.meeting_metadata.duration_ms,
                speakers,
                tags,
                template: "default".into(),
                dialect: self.dialect,
                panops_version: env!("CARGO_PKG_VERSION").into(),
                source_audio: input.meeting_metadata.source_path,
            },
            sections,
            language,
            generated_at: Utc::now(),
        })
    }
}

struct SectionDraft {
    title: String,
    narrative_md: String,
    key_points: Vec<String>,
    action_items: Vec<ActionItem>,
    time_range_ms: (u64, u64),
}

impl SectionDraft {
    fn from_json(time_range_ms: (u64, u64), _segs: Vec<Segment>, v: serde_json::Value) -> Self {
        let title = v
            .get("title")
            .and_then(|s| s.as_str())
            .unwrap_or("Untitled section")
            .to_string();
        let narrative_md = v
            .get("narrative_md")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let key_points = v
            .get("key_points")
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let action_items = v
            .get("action_items")
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| {
                        let o = x.as_object()?;
                        Some(ActionItem {
                            description: o.get("description")?.as_str()?.to_string(),
                            owner: o.get("owner").and_then(|v| v.as_str()).map(String::from),
                            due: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            title,
            narrative_md,
            key_points,
            action_items,
            time_range_ms,
        }
    }

    fn fallback(
        time_range_ms: (u64, u64),
        segs: Vec<Segment>,
        err: &str,
        dialect: MarkdownDialect,
    ) -> Self {
        let mut body = match dialect {
            MarkdownDialect::NotionEnhanced => format!("<!-- panops: llm error: {err} -->\n\n"),
            MarkdownDialect::Basic => format!("> panops: llm error: {err}\n\n"),
        };
        for seg in &segs {
            let label = match seg.speaker_id {
                Some(id) => format!("speaker_{id}"),
                None => "unknown".to_string(),
            };
            body.push_str(&format!("**{label}:** {}\n\n", seg.text));
        }
        Self {
            title: "Section".into(),
            narrative_md: body,
            key_points: vec![],
            action_items: vec![],
            time_range_ms,
        }
    }
}

fn dominant_language(segments: &[Segment]) -> String {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for s in segments {
        if let Some(l) = s.language_detected.as_deref() {
            *counts.entry(l).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(l, _)| l.to_string())
        .unwrap_or_else(|| "en".into())
}

fn collect_speakers(segments: &[Segment]) -> Vec<String> {
    let mut seen: HashSet<u32> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for s in segments {
        if let Some(id) = s.speaker_id {
            if seen.insert(id) {
                out.push(format!("speaker_{id}"));
            }
        }
    }
    out
}

fn extract_frontmatter(v: serde_json::Value) -> (String, Vec<String>) {
    let title = v
        .get("title")
        .and_then(|s| s.as_str())
        .unwrap_or("Untitled meeting")
        .to_string();
    let tags = v
        .get("tags")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    (title, tags)
}
