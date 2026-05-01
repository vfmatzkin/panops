//! End-to-end: NotesGenerator + MockLlm + canned responses match the committed
//! goldens at tests/fixtures/notes/multi_speaker_60s.expected.{json,*.md}.

use chrono::{FixedOffset, TimeZone, Utc};
use panops_core::Segment;
use panops_core::conformance::fakes::MockLlm;
use panops_core::exporter::NotesExporter;
use panops_core::llm::LlmResponse;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::input::{MeetingMetadata, NotesInput};
use panops_core::notes::pipeline::NotesGenerator;
use panops_core::notes::prompts::SectionSummary;
use panops_core::notes::prompts::build_frontmatter_prompt;
use panops_core::notes::prompts::build_section_narrative_prompt;
use panops_portable::markdown_exporter::MarkdownExporter;
use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("Cargo.toml").exists() && p.join("crates").exists())
        .unwrap()
        .to_path_buf()
}

fn fixture_segments() -> Vec<Segment> {
    vec![
        Segment {
            start_ms: 0,
            end_ms: 20_000,
            text: "Welcome to this meeting. Let's go over the agenda for today. \
                   We have several important items to discuss in the next sixty minutes together."
                .into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            is_partial: false,
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
            is_partial: false,
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
            is_partial: false,
            speaker_id: Some(0),
        },
    ]
}

fn canned_mock(dialect: MarkdownDialect) -> MockLlm {
    let segments = fixture_segments();
    let section_prompt = build_section_narrative_prompt(&segments, dialect, "en");
    let summaries = vec![SectionSummary {
        title: "Meeting kickoff and quarterly budget review".into(),
        key_points: vec![
            "Agenda includes budget review for next quarter".into(),
            "Spending plan needs approval before end of week".into(),
            "Review covers marketing, engineering, and operations line items".into(),
        ],
    }];
    let frontmatter_prompt = build_frontmatter_prompt(&summaries, "en", 60_000);
    MockLlm::default()
        .with_response_for(
            section_prompt.system.as_deref(),
            &section_prompt.user,
            LlmResponse::Json(serde_json::json!({
                "title": "Meeting kickoff and quarterly budget review",
                "narrative_md": "The meeting opened with a welcome and agenda overview \
                    covering the next sixty minutes. The first item was a budget review \
                    for next quarter, with approval required before week's end. The \
                    review walks through marketing line items first, then engineering, \
                    then any remaining operations expenses.",
                "key_points": [
                    "Agenda includes budget review for next quarter",
                    "Spending plan needs approval before end of week",
                    "Review covers marketing, engineering, and operations line items"
                ],
                "action_items": [
                    {"description": "Approve quarterly spending plan before end of week", "owner": null}
                ]
            })),
        )
        .with_response_for(
            frontmatter_prompt.system.as_deref(),
            &frontmatter_prompt.user,
            LlmResponse::Json(serde_json::json!({
                "title": "Quarterly budget review kickoff",
                "tags": ["budget-review", "quarterly", "kickoff"]
            })),
        )
}

fn run_pipeline(dialect: MarkdownDialect) -> panops_core::notes::ir::StructuredNotes {
    let mock = canned_mock(dialect);
    let generator = NotesGenerator {
        llm: &mock,
        dialect,
    };
    let mut notes = generator
        .generate(NotesInput {
            transcript: fixture_segments(),
            screenshots: vec![],
            meeting_metadata: MeetingMetadata {
                started_at: FixedOffset::east_opt(0)
                    .unwrap()
                    .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                    .unwrap(),
                duration_ms: 60_000,
                source_path: Some(PathBuf::from("tests/fixtures/audio/multi_speaker_60s.wav")),
                language_hint: Some("en".into()),
            },
        })
        .unwrap();
    notes.frontmatter.panops_version = "TESTING".into();
    notes.generated_at = Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap();
    notes
}

#[test]
fn ir_matches_golden() {
    let notes = run_pipeline(MarkdownDialect::NotionEnhanced);
    let actual = serde_json::to_string_pretty(&notes).unwrap();
    let expected = fs::read_to_string(
        workspace_root().join("tests/fixtures/notes/multi_speaker_60s.expected.json"),
    )
    .unwrap();
    assert_eq!(actual.trim(), expected.trim(), "IR drift");
}

#[test]
fn rendered_markdown_matches_notion_golden() {
    let notes = run_pipeline(MarkdownDialect::NotionEnhanced);
    let dir = tempfile::tempdir().unwrap();
    MarkdownExporter.export(&notes, dir.path()).unwrap();
    let actual = fs::read_to_string(dir.path().join("notes.md")).unwrap();
    let expected = fs::read_to_string(
        workspace_root().join("tests/fixtures/notes/multi_speaker_60s.expected.notion.md"),
    )
    .unwrap();
    assert_eq!(actual.trim(), expected.trim(), "notion markdown drift");
}

#[test]
fn rendered_markdown_matches_basic_golden() {
    let notes = run_pipeline(MarkdownDialect::Basic);
    let dir = tempfile::tempdir().unwrap();
    MarkdownExporter.export(&notes, dir.path()).unwrap();
    let actual = fs::read_to_string(dir.path().join("notes.md")).unwrap();
    let expected = fs::read_to_string(
        workspace_root().join("tests/fixtures/notes/multi_speaker_60s.expected.basic.md"),
    )
    .unwrap();
    assert_eq!(actual.trim(), expected.trim(), "basic markdown drift");
}
