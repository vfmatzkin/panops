//! Integration test: NotesGenerator end-to-end with MockLlm + canned segments.

use chrono::FixedOffset;
use chrono::TimeZone;
use panops_core::Segment;
use panops_core::conformance::fakes::MockLlm;
use panops_core::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::input::{MeetingMetadata, NotesInput};
use panops_core::notes::ir::Screenshot;
use panops_core::notes::pipeline::NotesGenerator;
use panops_core::notes::prompts::{
    SECTION_CHUNK_THRESHOLD_CHARS, SectionSummary, build_frontmatter_prompt,
    build_section_narrative_prompt,
};
use std::path::PathBuf;
use std::sync::Mutex;

fn seg(start: u64, end: u64, speaker: u32, text: &str) -> Segment {
    Segment {
        start_ms: start,
        end_ms: end,
        text: text.into(),
        language_detected: Some("en".into()),
        confidence: 1.0,
        is_partial: false,
        speaker_id: Some(speaker),
    }
}

fn make_mock(segments: &[Segment], duration_ms: u64) -> MockLlm {
    let section_prompt = build_section_narrative_prompt(segments, MarkdownDialect::Basic, "en");
    let frontmatter_prompt = build_frontmatter_prompt(
        &[SectionSummary {
            title: "Welcome".into(),
            key_points: vec!["meeting opened".into()],
        }],
        "en",
        duration_ms,
    );
    MockLlm::default()
        .with_response_for(
            section_prompt.system.as_deref(),
            &section_prompt.user,
            LlmResponse::Json(serde_json::json!({
                "title": "Welcome",
                "narrative_md": "The meeting opened with introductions.",
                "key_points": ["meeting opened"],
                "action_items": []
            })),
        )
        .with_response_for(
            frontmatter_prompt.system.as_deref(),
            &frontmatter_prompt.user,
            LlmResponse::Json(serde_json::json!({
                "title": "Team Meeting",
                "tags": ["meeting", "intro"]
            })),
        )
}

#[derive(Default)]
struct RecordingLlm {
    calls: Mutex<Vec<LlmRequest>>,
}

impl RecordingLlm {
    fn calls(&self) -> Vec<LlmRequest> {
        self.calls.lock().unwrap().clone()
    }
}

impl LlmProvider for RecordingLlm {
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let response = if req.user.starts_with("Section transcript") {
            let markers = marker_list(&req.user);
            let title = markers
                .first()
                .map_or_else(|| "Short Section".to_string(), |m| format!("Chunk {m}"));
            LlmResponse::Json(serde_json::json!({
                "title": title,
                "narrative_md": markers.join(" "),
                "key_points": markers,
                "action_items": []
            }))
        } else if req.user.starts_with("Sub-chunk summaries") {
            let markers = marker_list(&req.user);
            LlmResponse::Json(serde_json::json!({
                "title": "Long Section",
                "narrative_md": markers.join(" "),
                "key_points": markers,
                "action_items": []
            }))
        } else if req.user.starts_with("Section summaries") {
            LlmResponse::Json(serde_json::json!({
                "title": "Team Meeting",
                "tags": ["meeting"]
            }))
        } else {
            return Err(LlmError::Provider("unexpected prompt".into()));
        };
        self.calls.lock().unwrap().push(req);
        Ok(response)
    }
}

fn marker_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
        if token.starts_with("marker-") && !out.iter().any(|seen| seen == token) {
            out.push(token.to_string());
        }
    }
    out
}

fn notes_input(segments: Vec<Segment>, duration_ms: u64) -> NotesInput {
    NotesInput {
        transcript: segments,
        screenshots: vec![],
        meeting_metadata: MeetingMetadata {
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms,
            source_path: None,
            language_hint: Some("en".into()),
        },
    }
}

#[test]
fn one_section_pipeline_produces_structured_notes() {
    let segments = vec![seg(0, 60_000, 0, "hello and welcome to the meeting")];
    let mock = make_mock(&segments, 60_000);

    let generator = NotesGenerator {
        llm: &mock,
        dialect: MarkdownDialect::Basic,
    };
    let input = NotesInput {
        transcript: segments,
        screenshots: vec![],
        meeting_metadata: MeetingMetadata {
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 60_000,
            source_path: None,
            language_hint: Some("en".into()),
        },
    };

    let notes = generator.generate(input).expect("generate failed");
    assert_eq!(notes.sections.len(), 1);
    assert_eq!(notes.sections[0].title, "Welcome");
    assert_eq!(notes.frontmatter.title, "Team Meeting");
    assert_eq!(notes.frontmatter.tags, vec!["meeting", "intro"]);
    assert_eq!(notes.frontmatter.speakers, vec!["speaker_0"]);
}

#[test]
fn short_section_uses_single_section_llm_call_without_merge() {
    let segments = vec![seg(0, 60_000, 0, "marker-short hello and welcome")];
    let llm = RecordingLlm::default();
    let generator = NotesGenerator {
        llm: &llm,
        dialect: MarkdownDialect::Basic,
    };

    let notes = generator
        .generate(notes_input(segments, 60_000))
        .expect("generate failed");

    assert_eq!(notes.sections.len(), 1);
    assert_eq!(notes.sections[0].narrative_md, "marker-short");
    let calls = llm.calls();
    let section_calls = calls
        .iter()
        .filter(|c| c.user.starts_with("Section transcript"))
        .count();
    let merge_calls = calls
        .iter()
        .filter(|c| c.user.starts_with("Sub-chunk summaries"))
        .count();
    assert_eq!(section_calls, 1, "short sections keep the single-call path");
    assert_eq!(merge_calls, 0, "short sections must not run a merge pass");
}

#[test]
fn long_section_chunks_summarizes_each_chunk_and_merges_in_order() {
    let mut segments = Vec::new();
    let mut expected_markers = Vec::new();
    for i in 0..18u64 {
        let marker = format!("marker-{i:02}");
        expected_markers.push(marker.clone());
        let filler = " context".repeat(90);
        let start_ms = i * 4_000;
        segments.push(seg(
            start_ms,
            start_ms + 1_000,
            0,
            &format!("{marker}{filler}"),
        ));
    }

    let llm = RecordingLlm::default();
    let generator = NotesGenerator {
        llm: &llm,
        dialect: MarkdownDialect::Basic,
    };

    let notes = generator
        .generate(notes_input(segments, 72_000))
        .expect("generate failed");

    assert_eq!(notes.sections.len(), 1);
    let calls = llm.calls();
    let section_calls: Vec<_> = calls
        .iter()
        .filter(|c| c.user.starts_with("Section transcript"))
        .collect();
    let merge_calls: Vec<_> = calls
        .iter()
        .filter(|c| c.user.starts_with("Sub-chunk summaries"))
        .collect();

    assert!(
        section_calls.len() > 1,
        "long section should trigger multiple sub-chunk summaries"
    );
    assert_eq!(
        merge_calls.len(),
        1,
        "chunk summaries should be merged once"
    );
    for call in &section_calls {
        assert!(
            !marker_list(&call.user).is_empty(),
            "every sub-chunk summary prompt should contain transcript content"
        );
    }
    for call in &calls {
        assert!(
            call.user.len() <= SECTION_CHUNK_THRESHOLD_CHARS,
            "LLM input exceeded threshold: {} > {}",
            call.user.len(),
            SECTION_CHUNK_THRESHOLD_CHARS
        );
    }

    assert_eq!(notes.sections[0].title, "Long Section");
    let narrative = &notes.sections[0].narrative_md;
    for marker in &expected_markers {
        assert!(
            narrative.contains(marker),
            "final merged section should include {marker}; got {narrative}"
        );
    }
    let narrative_order: Vec<_> = expected_markers
        .iter()
        .map(|marker| narrative.find(marker).expect("marker should be present"))
        .collect();
    assert!(
        narrative_order.windows(2).all(|pair| pair[0] < pair[1]),
        "merged marker order should remain chronological: {narrative}"
    );
}

#[test]
fn verifier_replaces_section_narrative_when_llm_invents_speaker_id() {
    // Transcript only has speaker_0. LLM hallucinates speaker_99.
    // Verifier must catch the violation and the pipeline falls back to a
    // deterministic transcript dump for that section instead of shipping
    // the bogus narrative.
    let segments = vec![seg(0, 60_000, 0, "hello and welcome to the meeting")];

    let section_prompt = build_section_narrative_prompt(&segments, MarkdownDialect::Basic, "en");
    let frontmatter_prompt = build_frontmatter_prompt(
        &[SectionSummary {
            title: "Section".into(),
            key_points: vec![],
        }],
        "en",
        60_000,
    );
    let mock = MockLlm::default()
        .with_response_for(
            section_prompt.system.as_deref(),
            &section_prompt.user,
            LlmResponse::Json(serde_json::json!({
                "title": "Welcome",
                "narrative_md": "**speaker_99:** said something they did not say.",
                "key_points": [],
                "action_items": []
            })),
        )
        .with_response_for(
            frontmatter_prompt.system.as_deref(),
            &frontmatter_prompt.user,
            LlmResponse::Json(serde_json::json!({"title": "Meeting", "tags": []})),
        );

    let generator = NotesGenerator {
        llm: &mock,
        dialect: MarkdownDialect::Basic,
    };
    let input = NotesInput {
        transcript: segments,
        screenshots: vec![],
        meeting_metadata: MeetingMetadata {
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 60_000,
            source_path: None,
            language_hint: Some("en".into()),
        },
    };

    let notes = generator.generate(input).expect("generate failed");
    assert_eq!(notes.sections.len(), 1);
    let body = &notes.sections[0].narrative_md;
    assert!(
        !body.contains("speaker_99"),
        "verifier should have stripped the bogus speaker; got: {body}"
    );
    assert!(
        body.contains("verifier:") || body.contains("speaker_0:"),
        "fallback should emit a verifier marker and/or transcript dump; got: {body}"
    );
}

#[test]
fn frontmatter_llm_failure_falls_back_to_untitled() {
    let segments = vec![seg(0, 60_000, 0, "hello and welcome to the meeting")];

    let section_prompt = build_section_narrative_prompt(&segments, MarkdownDialect::Basic, "en");
    let frontmatter_prompt = build_frontmatter_prompt(
        &[SectionSummary {
            title: "Welcome".into(),
            key_points: vec!["meeting opened".into()],
        }],
        "en",
        60_000,
    );

    let mock = MockLlm::default()
        .with_response_for(
            section_prompt.system.as_deref(),
            &section_prompt.user,
            LlmResponse::Json(serde_json::json!({
                "title": "Welcome",
                "narrative_md": "The meeting opened with introductions.",
                "key_points": ["meeting opened"],
                "action_items": []
            })),
        )
        .with_error_for(
            frontmatter_prompt.system.as_deref(),
            &frontmatter_prompt.user,
            "simulated timeout",
        );

    let generator = NotesGenerator {
        llm: &mock,
        dialect: MarkdownDialect::Basic,
    };
    let input = NotesInput {
        transcript: segments,
        screenshots: vec![],
        meeting_metadata: MeetingMetadata {
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 60_000,
            source_path: None,
            language_hint: Some("en".into()),
        },
    };

    let notes = generator
        .generate(input)
        .expect("generate should not abort on frontmatter error");
    assert_eq!(notes.frontmatter.title, "Untitled meeting");
    assert!(notes.frontmatter.tags.is_empty());
    assert_eq!(
        notes.sections.len(),
        1,
        "section content should still be present"
    );
}

#[test]
fn screenshot_past_duration_is_clamped_to_last_section() {
    let segments = vec![seg(0, 60_000, 0, "hello and welcome to the meeting")];
    let mock = make_mock(&segments, 60_000);

    let generator = NotesGenerator {
        llm: &mock,
        dialect: MarkdownDialect::Basic,
    };
    // Screenshot at 90_000 ms exceeds duration_ms (60_000 ms).
    let input = NotesInput {
        transcript: segments,
        screenshots: vec![Screenshot {
            ms_since_start: 90_000,
            path: PathBuf::from("/tmp/late.jpg"),
            caption: None,
        }],
        meeting_metadata: MeetingMetadata {
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 60_000,
            source_path: None,
            language_hint: Some("en".into()),
        },
    };

    let notes = generator.generate(input).expect("generate failed");
    // Screenshot should be clamped and attached to the single section.
    assert_eq!(notes.sections.len(), 1);
    let shots = &notes.sections[0].screenshots;
    assert_eq!(
        shots.len(),
        1,
        "out-of-range screenshot should be preserved"
    );
    assert_eq!(
        shots[0].ms_since_start, 60_000,
        "timestamp should be clamped to duration_ms"
    );
}

#[test]
fn screenshot_with_duration_zero_passes_through_unclamped() {
    // duration_ms == 0 is malformed metadata. The clamp would collapse every
    // screenshot to ms=0, producing a degenerate output. Per #61, we skip the
    // clamp in that case (and emit a tracing::warn), letting the unclamped
    // values flow through to anchor_screenshots' nearest-section fallback.
    let segments = vec![seg(0, 60_000, 0, "hello and welcome to the meeting")];
    // Mock keyed on duration_ms=0 to match what NotesGenerator will request.
    let mock = make_mock(&segments, 0);

    let generator = NotesGenerator {
        llm: &mock,
        dialect: MarkdownDialect::Basic,
    };
    let input = NotesInput {
        transcript: segments,
        screenshots: vec![Screenshot {
            ms_since_start: 12_345,
            path: PathBuf::from("/tmp/img.jpg"),
            caption: None,
        }],
        meeting_metadata: MeetingMetadata {
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 0,
            source_path: None,
            language_hint: Some("en".into()),
        },
    };

    let notes = generator.generate(input).expect("generate failed");
    assert_eq!(notes.sections.len(), 1);
    let shots = &notes.sections[0].screenshots;
    assert_eq!(shots.len(), 1, "screenshot should be preserved");
    assert_eq!(
        shots[0].ms_since_start, 12_345,
        "timestamp must NOT be clamped to 0 when duration_ms == 0"
    );
}
