//! Integration test: NotesGenerator end-to-end with MockLlm + canned segments.

use chrono::FixedOffset;
use chrono::TimeZone;
use panops_core::Segment;
use panops_core::conformance::fakes::MockLlm;
use panops_core::llm::LlmResponse;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::input::{MeetingMetadata, NotesInput};
use panops_core::notes::ir::Screenshot;
use panops_core::notes::pipeline::NotesGenerator;
use panops_core::notes::prompts::{
    SectionSummary, build_frontmatter_prompt, build_section_narrative_prompt,
};
use std::path::PathBuf;

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
