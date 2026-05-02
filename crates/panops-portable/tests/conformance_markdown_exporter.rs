use std::fs;

use chrono::{FixedOffset, TimeZone, Utc};
use panops_core::exporter::NotesExporter;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::ir::{
    ActionItem, NotesFrontmatter, NotesSection, Screenshot, StructuredNotes,
};
use panops_portable::markdown_exporter::MarkdownExporter;

fn sample(dialect: MarkdownDialect) -> StructuredNotes {
    StructuredNotes {
        schema_version: StructuredNotes::SCHEMA_VERSION,
        frontmatter: NotesFrontmatter {
            title: "Test".into(),
            date: chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 60_000,
            speakers: vec!["speaker_0".into()],
            tags: vec!["test".into()],
            template: "default".into(),
            dialect,
            panops_version: "TESTING".into(),
            source_audio: None,
        },
        sections: vec![NotesSection {
            index: 1,
            title: "Section A".into(),
            time_range_ms: (0, 60_000),
            narrative_md: "Hello world.".into(),
            key_points: vec!["one".into()],
            action_items: vec![ActionItem {
                description: "do thing".into(),
                owner: None,
                due: None,
            }],
            screenshots: vec![],
        }],
        language: "en".into(),
        generated_at: Utc.with_ymd_and_hms(2026, 5, 1, 10, 1, 0).unwrap(),
    }
}

#[test]
fn exporter_writes_notes_md_and_returns_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let exporter = MarkdownExporter;
    let notes = sample(MarkdownDialect::Basic);
    let art = exporter.export(&notes, dir.path()).unwrap();
    assert_eq!(art.primary_file, dir.path().join("notes.md"));
    let body = fs::read_to_string(&art.primary_file).unwrap();
    assert!(body.starts_with("---\n"), "missing frontmatter fence");
    assert!(body.contains("title: Test"));
    assert!(body.contains("## 1. Section A"));
    assert!(body.contains("Hello world."));
}

#[test]
fn dialects_produce_different_output() {
    let dir = tempfile::tempdir().unwrap();
    let exporter = MarkdownExporter;
    exporter
        .export(&sample(MarkdownDialect::NotionEnhanced), dir.path())
        .unwrap();
    let notion_body = fs::read_to_string(dir.path().join("notes.md")).unwrap();

    let dir2 = tempfile::tempdir().unwrap();
    exporter
        .export(&sample(MarkdownDialect::Basic), dir2.path())
        .unwrap();
    let basic_body = fs::read_to_string(dir2.path().join("notes.md")).unwrap();

    assert!(notion_body.contains("dialect: notion-enhanced"));
    assert!(basic_body.contains("dialect: basic"));
    assert_ne!(notion_body, basic_body);
}

#[test]
fn screenshots_are_copied_into_dest_screenshots_dir_and_referenced_relatively() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = tempfile::tempdir().unwrap();
    let src_jpg = src_dir.path().join("frame.jpg");
    fs::write(&src_jpg, b"fake-jpeg").unwrap();

    let mut notes = sample(MarkdownDialect::Basic);
    notes.sections[0].screenshots.push(Screenshot {
        ms_since_start: 30_000,
        path: src_jpg.clone(),
        caption: None,
    });

    let exporter = MarkdownExporter;
    let art = exporter.export(&notes, dir.path()).unwrap();
    let copied = dir
        .path()
        .join("screenshots")
        .join("section01_00030000.jpg");
    assert!(copied.exists(), "screenshot not copied");
    assert!(art.assets.iter().any(|p| p == &copied));
    let body = fs::read_to_string(&art.primary_file).unwrap();
    assert!(body.contains("](screenshots/section01_00030000.jpg)"));
}

#[test]
fn empty_speakers_and_tags_emit_flow_style_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let exporter = MarkdownExporter;
    let mut notes = sample(MarkdownDialect::Basic);
    notes.frontmatter.speakers.clear();
    notes.frontmatter.tags.clear();
    let art = exporter.export(&notes, dir.path()).unwrap();
    let body = fs::read_to_string(&art.primary_file).unwrap();
    assert!(
        body.contains("speakers: []\n"),
        "expected 'speakers: []' but got:\n{body}"
    );
    assert!(
        body.contains("tags: []\n"),
        "expected 'tags: []' but got:\n{body}"
    );
    assert!(
        !body.contains("speakers:\ntags:"),
        "bare 'speakers:' key must not appear when list is empty"
    );
}

#[test]
fn single_quote_in_title_and_tags_is_double_quoted_in_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let exporter = MarkdownExporter;
    let mut notes = sample(MarkdownDialect::Basic);
    notes.frontmatter.title = "O'Reilly meeting".into();
    notes.frontmatter.tags = vec!["it's-great".into(), "plain".into()];
    let art = exporter.export(&notes, dir.path()).unwrap();
    let body = fs::read_to_string(&art.primary_file).unwrap();
    assert!(
        body.contains("title: \"O'Reilly meeting\""),
        "title with apostrophe must be double-quoted; got:\n{body}"
    );
    assert!(
        body.contains("  - \"it's-great\""),
        "tag with apostrophe must be double-quoted; got:\n{body}"
    );
    assert!(
        body.contains("  - plain"),
        "plain tag must remain unquoted; got:\n{body}"
    );
}

/// Producer for the slice 04 golden fixtures. Gated to avoid clobbering the
/// committed goldens on every test run; opt-in via PANOPS_REGEN_NOTES_GOLDENS=1.
#[test]
fn regenerate_multi_speaker_60s_goldens() {
    if std::env::var("PANOPS_REGEN_NOTES_GOLDENS").is_err() {
        eprintln!(
            "skipping regenerate_multi_speaker_60s_goldens: set PANOPS_REGEN_NOTES_GOLDENS=1"
        );
        return;
    }
    use chrono::FixedOffset;
    use chrono::TimeZone;
    use panops_core::Segment;
    use panops_core::conformance::fakes::MockLlm;
    use panops_core::llm::LlmResponse;
    use panops_core::notes::dialect::MarkdownDialect;
    use panops_core::notes::input::{MeetingMetadata, NotesInput};
    use panops_core::notes::pipeline::NotesGenerator;
    use panops_core::notes::prompts::{
        SectionSummary, build_frontmatter_prompt, build_section_narrative_prompt,
    };

    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("Cargo.toml").exists() && p.join("crates").exists())
        .unwrap()
        .to_path_buf();

    let segments = vec![
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
    ];

    let summaries = vec![SectionSummary {
        title: "Meeting kickoff and quarterly budget review".into(),
        key_points: vec![
            "Agenda includes budget review for next quarter".into(),
            "Spending plan needs approval before end of week".into(),
            "Review covers marketing, engineering, and operations line items".into(),
        ],
    }];
    let frontmatter_prompt = build_frontmatter_prompt(&summaries, "en", 60_000);

    let canned_section = serde_json::json!({
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
    });
    let canned_fm = serde_json::json!({
        "title": "Quarterly budget review kickoff",
        "tags": ["budget-review", "quarterly", "kickoff"]
    });

    let mock_for = |dialect: MarkdownDialect| -> MockLlm {
        let section_prompt = build_section_narrative_prompt(&segments, dialect, "en");
        MockLlm::default()
            .with_response_for(
                section_prompt.system.as_deref(),
                &section_prompt.user,
                LlmResponse::Json(canned_section.clone()),
            )
            .with_response_for(
                frontmatter_prompt.system.as_deref(),
                &frontmatter_prompt.user,
                LlmResponse::Json(canned_fm.clone()),
            )
    };

    let input = NotesInput {
        transcript: segments.clone(),
        screenshots: vec![],
        meeting_metadata: MeetingMetadata {
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 60_000,
            source_path: Some(std::path::PathBuf::from(
                "tests/fixtures/audio/multi_speaker_60s.wav",
            )),
            language_hint: Some("en".into()),
        },
    };

    let goldens_dir = workspace_root.join("tests/fixtures/notes");
    std::fs::create_dir_all(&goldens_dir).unwrap();

    for (dialect, file_suffix) in [
        (MarkdownDialect::NotionEnhanced, "notion"),
        (MarkdownDialect::Basic, "basic"),
    ] {
        let mock = mock_for(dialect);
        let generator = NotesGenerator {
            llm: &mock,
            dialect,
        };
        let mut notes = generator.generate(input.clone()).unwrap();
        notes.frontmatter.panops_version = "TESTING".into();
        notes.generated_at = chrono::Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap();

        if file_suffix == "notion" {
            std::fs::write(
                goldens_dir.join("multi_speaker_60s.expected.json"),
                serde_json::to_string_pretty(&notes).unwrap(),
            )
            .unwrap();
        }

        let dir = tempfile::tempdir().unwrap();
        let exporter = panops_portable::markdown_exporter::MarkdownExporter;
        let art =
            panops_core::exporter::NotesExporter::export(&exporter, &notes, dir.path()).unwrap();
        let body = std::fs::read_to_string(&art.primary_file).unwrap();
        std::fs::write(
            goldens_dir.join(format!("multi_speaker_60s.expected.{file_suffix}.md")),
            body,
        )
        .unwrap();
    }
}
