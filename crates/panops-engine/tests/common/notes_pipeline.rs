//! Deterministic ASR/diar/LLM/exporter wiring shared by every test
//! that exercises `notes.generate` end-to-end.
//!
//! The fakes here are tightly coupled: the segments returned by
//! `DeterministicAsr` MUST match the prompt fingerprints registered
//! on `build_mock_llm`'s output. If you change one, change both.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use panops_core::asr::{AsrError, AsrProvider};
use panops_core::conformance::fakes::MockLlm;
use panops_core::diar::{DiarError, Diarizer, SpeakerTurn};
use panops_core::llm::LlmResponse;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::prompts::{
    SectionSummary, build_frontmatter_prompt, build_section_narrative_prompt,
};
use panops_core::storage::Storage;
use panops_core::vad::{SpeechRegion, Vad, VadError};
use panops_core::{Segment, Transcript};
use panops_engine::server::EngineServices;
use panops_portable::markdown_exporter::MarkdownExporter;

/// Three-segment golden transcript used by the slice-04 regen test.
/// Reproduced verbatim here so the `MockLlm` prompt fingerprints
/// match the canned responses.
pub fn golden_segments() -> Vec<Segment> {
    vec![
        Segment {
            start_ms: 0,
            end_ms: 20_000,
            text: "Welcome to this meeting. Let's go over the agenda for today. \
                   We have several important items to discuss in the next sixty minutes together."
                .into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            speaker_id: Some(0),
        },
        Segment {
            start_ms: 20_000,
            end_ms: 40_000,
            text: "Thanks for the introduction. The first item is the budget review for next quarter. \
                   We need to approve the spending plan before the end of this week."
                .into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            speaker_id: Some(1),
        },
        Segment {
            start_ms: 40_000,
            end_ms: 60_000,
            text: "Right. I'll start with the marketing line items, then move to engineering, \
                   and finally we will cover any remaining operations expenses for the team."
                .into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            speaker_id: Some(0),
        },
    ]
}

pub struct DeterministicAsr;

impl AsrProvider for DeterministicAsr {
    fn transcribe(
        &self,
        _samples: &[f32],
        _sample_rate: u32,
        _language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        Ok(Transcript {
            schema_version: Transcript::SCHEMA_VERSION,
            model: "deterministic-asr".into(),
            audio_path: std::path::PathBuf::new(),
            audio_duration_ms: 60_000,
            diarized: false,
            segments: golden_segments(),
        })
    }

    fn is_fake(&self) -> bool {
        true
    }
}

pub struct DeterministicDiar;

impl Diarizer for DeterministicDiar {
    fn diarize(&self, _audio_path: &Path) -> Result<Vec<SpeakerTurn>, DiarError> {
        Ok(vec![
            SpeakerTurn {
                start_ms: 0,
                end_ms: 20_000,
                speaker_id: 0,
            },
            SpeakerTurn {
                start_ms: 20_000,
                end_ms: 40_000,
                speaker_id: 1,
            },
            SpeakerTurn {
                start_ms: 40_000,
                end_ms: 60_000,
                speaker_id: 0,
            },
        ])
    }

    fn is_fake(&self) -> bool {
        true
    }
}

pub fn build_mock_llm(dialect: MarkdownDialect) -> MockLlm {
    let segments = golden_segments();
    let canned_section = serde_json::json!({
        "title": "Meeting kickoff and quarterly budget review",
        "narrative_md": "The session opened with a welcome and a brief handoff \
            into the agenda. The first agenda item framed the rest of the \
            meeting, with the discussion organising the review into a clear \
            sequence so each functional area would get its own slot.",
        "key_points": [
            "Budget review scoped to next quarter only",
            "Review sequence: marketing, engineering, operations"
        ],
        "action_items": [
            {"description": "Approve quarterly spending plan before end of week", "owner": null}
        ]
    });
    let canned_fm = serde_json::json!({
        "title": "Quarterly budget review kickoff",
        "tags": ["budget-review", "quarterly", "kickoff"]
    });
    let summaries = vec![SectionSummary {
        title: "Meeting kickoff and quarterly budget review".into(),
        key_points: vec![
            "Budget review scoped to next quarter only".into(),
            "Review sequence: marketing, engineering, operations".into(),
        ],
    }];
    let section_prompt = build_section_narrative_prompt(&segments, dialect, "en");
    let frontmatter_prompt = build_frontmatter_prompt(&summaries, "en", 60_000);
    MockLlm::default()
        .with_response_for(
            section_prompt.system.as_deref(),
            &section_prompt.user,
            LlmResponse::Json(canned_section),
        )
        .with_response_for(
            frontmatter_prompt.system.as_deref(),
            &frontmatter_prompt.user,
            LlmResponse::Json(canned_fm),
        )
}

/// Deterministic VAD fake that always returns a single region covering
/// the entire audio. Used in IPC tests where splitting into multiple
/// regions would duplicate the deterministic ASR output.
pub struct SingleRegionVad;

impl Vad for SingleRegionVad {
    fn detect_speech(
        &self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<SpeechRegion>, VadError> {
        let duration_ms = (samples.len() as u64 * 1000) / u64::from(sample_rate);
        Ok(vec![SpeechRegion {
            start_ms: 0,
            end_ms: duration_ms,
        }])
    }
}

/// Build an `EngineServices` with a deterministic ASR + diar + LLM
/// pipeline and a real `MarkdownExporter`. Uses dialect = `Basic` so
/// the prompts are simpler and the goldens are short.
pub fn build_deterministic_notes_services(
    storage: Arc<dyn Storage>,
    data_dir: std::path::PathBuf,
) -> EngineServices {
    EngineServices::ready(
        Arc::new(build_mock_llm(MarkdownDialect::Basic)),
        storage,
        data_dir,
        Arc::new(DeterministicAsr),
        Arc::new(DeterministicDiar),
        Arc::new(MarkdownExporter),
        Arc::new(SingleRegionVad),
    )
}
