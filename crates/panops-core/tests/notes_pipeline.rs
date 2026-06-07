//! Integration test: NotesGenerator end-to-end with MockLlm + canned segments.

use chrono::FixedOffset;
use chrono::TimeZone;
use panops_core::Segment;
use panops_core::conformance::fakes::MockLlm;
use panops_core::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::input::{MeetingMetadata, NotesInput};
use panops_core::notes::ir::Screenshot;
use panops_core::notes::pipeline::{NotesGenerator, rendered_llm_request_chars};
use panops_core::notes::prompts::{
    SECTION_CHUNK_THRESHOLD_CHARS, SectionSummary, build_frontmatter_prompt,
    build_section_narrative_prompt, estimate_transcript_chars,
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

#[derive(Default)]
struct BulkyRecordingLlm {
    calls: Mutex<Vec<LlmRequest>>,
}

impl BulkyRecordingLlm {
    fn calls(&self) -> Vec<LlmRequest> {
        self.calls.lock().unwrap().clone()
    }
}

impl LlmProvider for BulkyRecordingLlm {
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let response = if req.user.starts_with("Section transcript") {
            let markers = marker_list(&req.user);
            LlmResponse::Json(serde_json::json!({
                "title": markers.first().map_or("Chunk", String::as_str),
                "narrative_md": format!("{} {}", markers.join(" "), "detail ".repeat(300)),
                "key_points": markers,
                "action_items": []
            }))
        } else if req.user.starts_with("Sub-chunk summaries") {
            let markers = marker_list(&req.user);
            LlmResponse::Json(serde_json::json!({
                "title": "Merged Section",
                "narrative_md": format!("{} {}", markers.join(" "), "detail ".repeat(300)),
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

#[derive(Default)]
struct MergeFailingLlm {
    calls: Mutex<Vec<LlmRequest>>,
}

impl LlmProvider for MergeFailingLlm {
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let response = if req.user.starts_with("Section transcript") {
            let markers = marker_list(&req.user);
            LlmResponse::Json(serde_json::json!({
                "title": markers.first().map_or("Chunk", String::as_str),
                "narrative_md": markers.join(" "),
                "key_points": markers,
                "action_items": []
            }))
        } else if req.user.starts_with("Sub-chunk summaries") {
            LlmResponse::Text("merge failed as text".into())
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
fn near_threshold_rendered_section_uses_chunk_path() {
    let mut segments = Vec::new();
    let segment_count = 20usize;
    let mut remaining_estimate = SECTION_CHUNK_THRESHOLD_CHARS - 16;

    for i in 0..segment_count {
        let marker = format!("marker-rendered-{i:02}");
        let remaining_segments = segment_count - i;
        let line_chars = remaining_estimate / remaining_segments;
        let text_len = line_chars.saturating_sub(20);
        let filler_len = text_len.saturating_sub(marker.len() + 1);
        let text = format!("{marker} {}", "x".repeat(filler_len));
        remaining_estimate = remaining_estimate.saturating_sub(text.len() + 20);
        let start_ms = i as u64 * 4_000;
        segments.push(seg(start_ms, start_ms + 1_000, 0, &text));
    }

    let transcript_chars = estimate_transcript_chars(&segments);
    let single_req = build_section_narrative_prompt(&segments, MarkdownDialect::Basic, "en");
    assert!(
        transcript_chars < SECTION_CHUNK_THRESHOLD_CHARS,
        "fixture must stay under the transcript-only threshold: {transcript_chars}"
    );
    assert!(
        rendered_llm_request_chars(&single_req) > SECTION_CHUNK_THRESHOLD_CHARS,
        "fixture must exceed the rendered prompt threshold"
    );

    let llm = RecordingLlm::default();
    let generator = NotesGenerator {
        llm: &llm,
        dialect: MarkdownDialect::Basic,
    };

    let notes = generator
        .generate(notes_input(segments, 80_000))
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
        "rendered prompt overhead should force multiple chunk summaries"
    );
    assert_eq!(
        merge_calls.len(),
        1,
        "multiple rendered-budget chunks should merge once"
    );
}

#[test]
fn single_chunk_long_section_skips_merge_call() {
    let segments = vec![seg(
        0,
        60_000,
        0,
        &format!("marker-single-chunk {}", "context ".repeat(1_200)),
    )];
    let single_req = build_section_narrative_prompt(&segments, MarkdownDialect::Basic, "en");
    assert!(
        rendered_llm_request_chars(&single_req) > SECTION_CHUNK_THRESHOLD_CHARS,
        "fixture must exceed the rendered prompt threshold"
    );

    let llm = RecordingLlm::default();
    let generator = NotesGenerator {
        llm: &llm,
        dialect: MarkdownDialect::Basic,
    };

    let notes = generator
        .generate(notes_input(segments, 60_000))
        .expect("generate failed");

    assert_eq!(notes.sections.len(), 1);
    assert_eq!(notes.sections[0].narrative_md, "marker-single-chunk");
    let calls = llm.calls();
    let section_calls = calls
        .iter()
        .filter(|c| c.user.starts_with("Section transcript"))
        .count();
    let merge_calls = calls
        .iter()
        .filter(|c| c.user.starts_with("Sub-chunk summaries"))
        .count();
    assert_eq!(
        section_calls, 1,
        "single unbreakable chunk is summarized once"
    );
    assert_eq!(
        merge_calls, 0,
        "one chunk summary should pass through without an LLM merge"
    );
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
            rendered_llm_request_chars(call) <= SECTION_CHUNK_THRESHOLD_CHARS,
            "LLM input exceeded threshold: {} > {}",
            rendered_llm_request_chars(call),
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
fn long_section_merges_chunk_summaries_in_bounded_rounds() {
    let mut segments = Vec::new();
    let mut expected_markers = Vec::new();
    for i in 0..12u64 {
        let marker = format!("marker-bounded-{i:02}");
        expected_markers.push(marker.clone());
        let filler = " chunk".repeat(620);
        let start_ms = i * 4_000;
        segments.push(seg(
            start_ms,
            start_ms + 1_000,
            0,
            &format!("{marker}{filler}"),
        ));
    }

    let llm = BulkyRecordingLlm::default();
    let generator = NotesGenerator {
        llm: &llm,
        dialect: MarkdownDialect::Basic,
    };

    let notes = generator
        .generate(notes_input(segments, 48_000))
        .expect("generate failed");

    assert_eq!(notes.sections.len(), 1);
    assert_eq!(notes.sections[0].title, "Merged Section");
    assert!(
        !notes.sections[0].narrative_md.contains("panops: llm error"),
        "iterative merge should produce notes without falling back"
    );

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
        "fixture should trigger chunk-level summaries"
    );
    assert!(
        merge_calls.len() > 1,
        "bulky chunk summaries should require multiple bounded merge calls"
    );
    assert!(
        merge_calls
            .iter()
            .any(|call| call.user.contains("Title: Merged Section")),
        "at least one merge call should consume a previous merge result"
    );

    for call in calls.iter().filter(|call| {
        call.user.starts_with("Section transcript") || call.user.starts_with("Sub-chunk summaries")
    }) {
        assert!(
            rendered_llm_request_chars(call) <= SECTION_CHUNK_THRESHOLD_CHARS,
            "LLM input exceeded threshold: {} > {}\n{}",
            rendered_llm_request_chars(call),
            SECTION_CHUNK_THRESHOLD_CHARS,
            call.user
        );
    }

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
fn long_section_merge_text_response_falls_back_without_aborting() {
    let mut segments = Vec::new();
    for i in 0..18u64 {
        let marker = format!("marker-fallback-{i:02}");
        let filler = " context".repeat(90);
        let start_ms = i * 4_000;
        segments.push(seg(
            start_ms,
            start_ms + 1_000,
            0,
            &format!("{marker}{filler}"),
        ));
    }

    let llm = MergeFailingLlm::default();
    let generator = NotesGenerator {
        llm: &llm,
        dialect: MarkdownDialect::Basic,
    };

    let notes = generator
        .generate(notes_input(segments, 72_000))
        .expect("generate should not abort when merge falls back");

    assert_eq!(notes.sections.len(), 1);
    assert_eq!(notes.sections[0].title, "Section");
    let narrative = &notes.sections[0].narrative_md;
    assert!(
        narrative.contains("panops: llm error: LLM unavailable"),
        "fallback should include a generic LLM marker; got: {narrative}"
    );
    assert!(
        !narrative.contains("merge failed as text")
            && !narrative.contains("merge LLM returned text, expected json"),
        "fallback should not leak internal LLM details; got: {narrative}"
    );
    assert!(
        narrative.contains("marker-fallback-00"),
        "fallback should preserve original transcript content; got: {narrative}"
    );
    assert_eq!(notes.frontmatter.title, "Team Meeting");
}

#[test]
fn all_sections_failing_llm_returns_error_not_empty_notes() {
    use panops_core::conformance::fakes::FailingLlm;
    use panops_core::notes::error::NotesError;

    // Provider unavailable at runtime (e.g. FoundationModels with Apple
    // Intelligence off): every `complete` call errors. With no successful
    // section narrative, the notes file would be all `## N. Section` stubs.
    // The pipeline must surface a clear error instead of reporting success.
    let segments = vec![
        seg(0, 60_000, 0, "hello and welcome to the meeting"),
        seg(60_000, 120_000, 1, "let us discuss the quarterly roadmap"),
    ];
    let llm = FailingLlm::provider("Model is unavailable. Apple Intelligence is not enabled.");
    let generator = NotesGenerator {
        llm: &llm,
        dialect: MarkdownDialect::Basic,
    };

    let err = generator
        .generate(notes_input(segments, 120_000))
        .expect_err("provider down on every call must not yield empty-section notes as success");

    assert!(
        matches!(err, NotesError::LlmUnavailable { .. }),
        "expected LlmUnavailable, got {err:?}"
    );
    assert!(
        err.to_string().contains("unavailable"),
        "error message should be actionable; got: {err}"
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
