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
//! A failed frontmatter LLM call falls back to `title = "Untitled meeting"`
//! and an empty tag list; the pipeline does not abort.

use std::collections::HashSet;

use chrono::Utc;
use rayon::prelude::*;

use crate::Segment;
use crate::llm::{LlmProvider, LlmRequest, LlmResponse};
use crate::notes::dialect::MarkdownDialect;
use crate::notes::error::NotesError;
use crate::notes::input::NotesInput;
use crate::notes::ir::{ActionItem, NotesFrontmatter, NotesSection, Screenshot, StructuredNotes};
use crate::notes::prompts::{
    SECTION_CHUNK_TARGET_CHARS, SECTION_CHUNK_THRESHOLD_CHARS, SectionSummary,
    build_frontmatter_prompt, build_merge_section_prompt, build_section_narrative_prompt,
    estimate_chunk_summary_chars, estimate_transcript_chars, split_segments_for_chunking,
};
use crate::notes::screenshot_anchoring::anchor_screenshots;
use crate::notes::topic_segmentation::{TopicSegmentationConfig, segment_topics};
use crate::notes::verifier;
use serde_json::Value;

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
        let unique_languages = unique_languages(&input.transcript);

        // Stage 1
        let raw_sections = segment_topics(&input.transcript, &TopicSegmentationConfig::default());

        // Build the allowed-speaker set once for the verifier (Stage 2).
        let allowed_speakers: HashSet<u32> = input
            .transcript
            .iter()
            .filter_map(|s| s.speaker_id)
            .collect();

        // Stage 2 (parallel)
        let section_drafts: Vec<SectionDraft> = raw_sections
            .par_iter()
            .map(|raw| {
                let segs: Vec<Segment> = raw
                    .segment_indices
                    .iter()
                    .map(|i| input.transcript[*i].clone())
                    .collect();
                self.process_section(raw.time_range_ms, segs, &language, &allowed_speakers)
            })
            .collect();

        // Stage 3 — clamp out-of-range timestamps, then anchor.
        // duration_ms == 0 is a malformed-input edge case (e.g. all-zero-duration
        // segments with a non-empty transcript). In that state every clamp
        // collapses to 0, producing degenerate output. Skip the clamp + warn
        // rather than silently zero everything; downstream anchor still runs
        // against unclamped values, which the fallback midpoint logic handles.
        let duration_ms = input.meeting_metadata.duration_ms;
        let clamped_screenshots: Vec<Screenshot> = if duration_ms == 0 {
            if !input.screenshots.is_empty() {
                tracing::warn!(
                    n = input.screenshots.len(),
                    "duration_ms == 0; skipping screenshot clamp (malformed metadata)"
                );
            }
            input.screenshots.clone()
        } else {
            input
                .screenshots
                .iter()
                .map(|s| Screenshot {
                    ms_since_start: s.ms_since_start.min(duration_ms),
                    ..s.clone()
                })
                .collect()
        };
        let per_section_screenshots = anchor_screenshots(&raw_sections, &clamped_screenshots);

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
        let (title, tags) = match self.llm.complete(fm_req) {
            Ok(LlmResponse::Json(v)) => extract_frontmatter(v),
            Ok(LlmResponse::Text(_)) => {
                tracing::warn!(
                    "frontmatter LLM call returned text instead of JSON; using defaults"
                );
                ("Untitled meeting".to_string(), Vec::new())
            }
            Err(e) => {
                tracing::warn!(error = %e, "frontmatter LLM call failed; using defaults");
                ("Untitled meeting".to_string(), Vec::new())
            }
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
                languages: unique_languages,
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

    /// Process a single section: either single LLM call (if transcript fits)
    /// or chunked multi-call with merge (if transcript exceeds threshold).
    fn process_section(
        &self,
        time_range_ms: (u64, u64),
        segs: Vec<Segment>,
        language: &str,
        allowed_speakers: &HashSet<u32>,
    ) -> SectionDraft {
        let transcript_chars = estimate_transcript_chars(&segs);
        let single_req = build_section_narrative_prompt(&segs, self.dialect, language);
        let single_req_chars = rendered_llm_request_chars(&single_req);

        if single_req_chars <= SECTION_CHUNK_THRESHOLD_CHARS {
            // Normal case: single LLM call
            self.process_single_section(time_range_ms, segs, single_req, allowed_speakers)
        } else {
            // Long section: chunk, summarize each, merge
            tracing::info!(
                time_range = ?time_range_ms,
                transcript_chars,
                rendered_chars = single_req_chars,
                threshold = SECTION_CHUNK_THRESHOLD_CHARS,
                "section exceeds chunk threshold; splitting"
            );
            // Split toward the lower soft target, while keeping the threshold
            // as the hard transcript ceiling. Rendered prompt overhead is
            // checked below because system/schema/dialect text also count.
            let initial_chunks = split_segments_for_chunking(
                &segs,
                SECTION_CHUNK_TARGET_CHARS,
                SECTION_CHUNK_THRESHOLD_CHARS,
            );
            let chunks = split_chunks_to_rendered_budget(
                initial_chunks,
                self.dialect,
                language,
                SECTION_CHUNK_THRESHOLD_CHARS,
            );
            let mut chunk_summaries = Vec::with_capacity(chunks.len());
            for (i, chunk) in chunks.iter().enumerate() {
                let req = build_section_narrative_prompt(chunk, self.dialect, language);
                match self.llm.complete(req) {
                    Ok(LlmResponse::Json(v)) => chunk_summaries.push(v),
                    Ok(LlmResponse::Text(_)) => {
                        tracing::warn!(
                            chunk_index = i + 1,
                            "chunk summary LLM returned text, expected json"
                        );
                        return SectionDraft::fallback(
                            time_range_ms,
                            segs,
                            "LLM unavailable",
                            self.dialect,
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            chunk_index = i + 1,
                            error = %e,
                            "chunk summary LLM call failed"
                        );
                        return SectionDraft::fallback(
                            time_range_ms,
                            segs,
                            "LLM unavailable",
                            self.dialect,
                        );
                    }
                }
            }

            // Merge chunk summaries into final section. The merge itself is
            // bounded so many sub-chunks cannot recreate the original
            // over-context prompt.
            match self.merge_chunk_summaries(chunk_summaries, language) {
                Ok(v) => {
                    let draft = SectionDraft::from_json(time_range_ms, v);
                    match verifier::verify_section_attribution(
                        &draft.narrative_md,
                        &draft.action_items,
                        allowed_speakers,
                    ) {
                        verifier::VerifierReport::Ok => draft,
                        verifier::VerifierReport::DisallowedSpeakers(ids) => {
                            tracing::warn!(
                                section_ms = ?time_range_ms,
                                disallowed = ?ids,
                                "merged section referenced speakers not in transcript; using fallback"
                            );
                            SectionDraft::fallback(
                                time_range_ms,
                                segs,
                                "verifier: disallowed speaker reference",
                                self.dialect,
                            )
                        }
                    }
                }
                Err(err) => SectionDraft::fallback(time_range_ms, segs, &err, self.dialect),
            }
        }
    }

    fn merge_chunk_summaries(
        &self,
        mut summaries: Vec<Value>,
        language: &str,
    ) -> Result<Value, String> {
        if summaries.is_empty() {
            return Err("merge LLM call failed: no chunk summaries".to_string());
        }
        if summaries.len() == 1 {
            return Ok(summaries.remove(0));
        }

        while summaries.len() > 1 {
            let before_estimate = summaries
                .iter()
                .map(estimate_chunk_summary_chars)
                .sum::<usize>();
            let mut next_round = Vec::new();
            let mut index = 0;

            while index < summaries.len() {
                let mut batch = Vec::new();

                while index < summaries.len() {
                    let next = summaries[index].clone();

                    if !batch.is_empty() {
                        let mut candidate_batch = batch.clone();
                        candidate_batch.push(next.clone());
                        let candidate_req =
                            build_merge_section_prompt(&candidate_batch, self.dialect, language);
                        if rendered_llm_request_chars(&candidate_req)
                            > SECTION_CHUNK_THRESHOLD_CHARS
                        {
                            break;
                        }
                    }

                    batch.push(next);
                    index += 1;
                }

                if batch.len() == 1 {
                    next_round.push(batch.remove(0));
                    continue;
                }

                let merge_req = build_merge_section_prompt(&batch, self.dialect, language);
                let merge_req_chars = rendered_llm_request_chars(&merge_req);
                if merge_req_chars > SECTION_CHUNK_THRESHOLD_CHARS {
                    tracing::warn!(
                        input_chars = merge_req_chars,
                        threshold = SECTION_CHUNK_THRESHOLD_CHARS,
                        "single chunk summary exceeds merge context threshold"
                    );
                }

                match self.llm.complete(merge_req) {
                    Ok(LlmResponse::Json(v)) => next_round.push(v),
                    Ok(LlmResponse::Text(_)) => {
                        tracing::warn!("merge LLM returned text, expected json");
                        return Err("LLM unavailable".to_string());
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "merge LLM call failed");
                        return Err("LLM unavailable".to_string());
                    }
                }
            }

            let after_estimate = next_round
                .iter()
                .map(estimate_chunk_summary_chars)
                .sum::<usize>();
            if next_round.len() == summaries.len() && after_estimate >= before_estimate {
                return Err(
                    "merge LLM call failed: chunk summaries cannot be reduced within context threshold"
                        .to_string(),
                );
            }
            summaries = next_round;
        }

        Ok(summaries.remove(0))
    }

    /// Single-section LLM call (original path, unchanged).
    fn process_single_section(
        &self,
        time_range_ms: (u64, u64),
        segs: Vec<Segment>,
        req: LlmRequest,
        allowed_speakers: &HashSet<u32>,
    ) -> SectionDraft {
        match self.llm.complete(req) {
            Ok(LlmResponse::Json(v)) => {
                let draft = SectionDraft::from_json(time_range_ms, v);
                match verifier::verify_section_attribution(
                    &draft.narrative_md,
                    &draft.action_items,
                    allowed_speakers,
                ) {
                    verifier::VerifierReport::Ok => draft,
                    verifier::VerifierReport::DisallowedSpeakers(ids) => {
                        tracing::warn!(
                            section_ms = ?time_range_ms,
                            disallowed = ?ids,
                            "section narrative referenced speakers not in transcript; using fallback"
                        );
                        SectionDraft::fallback(
                            time_range_ms,
                            segs,
                            "verifier: disallowed speaker reference",
                            self.dialect,
                        )
                    }
                }
            }
            Ok(LlmResponse::Text(_)) => SectionDraft::fallback(
                time_range_ms,
                segs,
                "llm returned text, expected json",
                self.dialect,
            ),
            Err(e) => SectionDraft::fallback(time_range_ms, segs, &e.to_string(), self.dialect),
        }
    }
}

fn split_chunks_to_rendered_budget(
    chunks: Vec<Vec<Segment>>,
    dialect: MarkdownDialect,
    language: &str,
    max_chars: usize,
) -> Vec<Vec<Segment>> {
    let mut out = Vec::new();
    for chunk in chunks {
        push_rendered_budget_chunk(chunk, dialect, language, max_chars, &mut out);
    }
    out
}

fn push_rendered_budget_chunk(
    chunk: Vec<Segment>,
    dialect: MarkdownDialect,
    language: &str,
    max_chars: usize,
    out: &mut Vec<Vec<Segment>>,
) {
    let req = build_section_narrative_prompt(&chunk, dialect, language);
    if chunk.len() <= 1 || rendered_llm_request_chars(&req) <= max_chars {
        out.push(chunk);
        return;
    }

    let mut current = Vec::new();
    for seg in chunk {
        current.push(seg);
        let req = build_section_narrative_prompt(&current, dialect, language);
        if rendered_llm_request_chars(&req) > max_chars && current.len() > 1 {
            let overflow = current.pop().expect("current has at least two segments");
            out.push(current);
            current = vec![overflow];
        }
    }

    if !current.is_empty() {
        push_rendered_budget_chunk(current, dialect, language, max_chars, out);
    }
}

#[doc(hidden)]
pub fn rendered_llm_request_chars(req: &LlmRequest) -> usize {
    req.system.as_deref().map_or(0, str::len)
        + req.user.len()
        + req
            .schema
            .as_ref()
            .map_or(0, |schema| schema.to_string().len())
}

struct SectionDraft {
    title: String,
    narrative_md: String,
    key_points: Vec<String>,
    action_items: Vec<ActionItem>,
    time_range_ms: (u64, u64),
}

impl SectionDraft {
    fn from_json(time_range_ms: (u64, u64), v: serde_json::Value) -> Self {
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

fn unique_languages(segments: &[Segment]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for s in segments {
        if let Some(lang) = s.language_detected.as_deref() {
            if seen.insert(lang.to_string()) {
                out.push(lang.to_string());
            }
        }
    }
    if out.is_empty() {
        vec!["en".into()]
    } else {
        out
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Segment;

    #[test]
    fn unique_languages_preserves_first_appearance_order() {
        let segs = vec![
            Segment {
                start_ms: 0,
                end_ms: 1000,
                text: "hello".into(),
                language_detected: Some("en".into()),
                confidence: 1.0,
                is_partial: false,
                speaker_id: Some(0),
            },
            Segment {
                start_ms: 1000,
                end_ms: 2000,
                text: "hola".into(),
                language_detected: Some("es".into()),
                confidence: 1.0,
                is_partial: false,
                speaker_id: Some(1),
            },
            Segment {
                start_ms: 2000,
                end_ms: 3000,
                text: "world".into(),
                language_detected: Some("en".into()),
                confidence: 1.0,
                is_partial: false,
                speaker_id: Some(0),
            },
            Segment {
                start_ms: 3000,
                end_ms: 4000,
                text: "mundo".into(),
                language_detected: Some("es".into()),
                confidence: 1.0,
                is_partial: false,
                speaker_id: Some(1),
            },
        ];
        let langs = unique_languages(&segs);
        assert_eq!(langs, vec!["en", "es"]);
    }

    #[test]
    fn unique_languages_defaults_to_en_when_none_detected() {
        let segs = vec![Segment {
            start_ms: 0,
            end_ms: 1000,
            text: "hello".into(),
            language_detected: None,
            confidence: 1.0,
            is_partial: false,
            speaker_id: None,
        }];
        let langs = unique_languages(&segs);
        assert_eq!(langs, vec!["en"]);
    }

    #[test]
    fn unique_languages_empty_transcript_defaults_to_en() {
        let langs = unique_languages(&[]);
        assert_eq!(langs, vec!["en"]);
    }
}
