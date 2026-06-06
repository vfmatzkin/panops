//! Golden-snapshot regression tests for the notes pipeline's LLM prompts.
//!
//! CI exercises the notes pipeline only via `MockLlm`, which returns canned
//! responses regardless of prompt content. This test catches regressions in
//! the rendered prompt text (system, user, schema) by comparing against
//! committed golden files.
//!
//! Run with `PANOPS_REGEN_PROMPT_GOLDENS=1` to regenerate goldens after
//! intentional prompt changes.

use panops_core::Segment;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::prompts::{
    SectionSummary, build_frontmatter_prompt, build_merge_section_prompt,
    build_section_narrative_prompt,
};

/// Fixed sample transcript for deterministic golden generation.
/// Two segments, one with speaker_0, one with speaker_1.
fn sample_segments() -> Vec<Segment> {
    vec![
        Segment {
            start_ms: 0,
            end_ms: 5000,
            text: "Hello, let's discuss the quarterly review.".into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            is_partial: false,
            speaker_id: Some(0),
        },
        Segment {
            start_ms: 5000,
            end_ms: 12_000,
            text: "Thanks. I'll cover the budget items first.".into(),
            language_detected: Some("en".into()),
            confidence: 1.0,
            is_partial: false,
            speaker_id: Some(1),
        },
    ]
}

/// Fixed sample summaries for frontmatter and merge prompts.
fn sample_summaries() -> Vec<SectionSummary> {
    vec![SectionSummary {
        title: "Quarterly budget review kickoff".into(),
        key_points: vec![
            "Budget scoped to next quarter".into(),
            "Review sequence: marketing, engineering".into(),
        ],
    }]
}

/// Fixed sample chunk summaries for merge prompt.
fn sample_chunk_summaries() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "title": "Part 1: Introduction",
            "narrative_md": "The meeting opened with a welcome and agenda overview.",
            "key_points": ["Budget review scoped to Q2"],
            "action_items": []
        }),
        serde_json::json!({
            "title": "Part 2: Budget discussion",
            "narrative_md": "The team reviewed the marketing and engineering budgets.",
            "key_points": ["Marketing budget approved", "Engineering deferred"],
            "action_items": [{"description": "Finalize engineering budget", "owner": null}]
        }),
    ]
}

/// Find workspace root for golden file paths.
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("Cargo.toml").exists() && p.join("crates").exists())
        .unwrap()
        .to_path_buf()
}

/// Serialize a prompt (system + user + schema) to a stable string.
/// Format: three sections separated by `---` fences.
fn serialize_prompt(
    system: Option<&str>,
    user: &str,
    schema: Option<&serde_json::Value>,
) -> String {
    let mut out = String::new();
    out.push_str("=== SYSTEM ===\n");
    if let Some(s) = system {
        out.push_str(s);
        out.push('\n');
    } else {
        out.push_str("(none)\n");
    }
    out.push_str("\n=== USER ===\n");
    out.push_str(user);
    out.push('\n');
    out.push_str("\n=== SCHEMA ===\n");
    if let Some(s) = schema {
        out.push_str(&serde_json::to_string_pretty(s).unwrap());
        out.push('\n');
    } else {
        out.push_str("(none)\n");
    }
    out
}

/// Check/generate golden for a single prompt. Returns true if mismatch detected.
fn check_or_regenerate_golden(
    name: &str,
    actual: &str,
    goldens_dir: &std::path::Path,
    regen: bool,
) -> bool {
    let golden_path = goldens_dir.join(format!("{name}.golden.txt"));
    if regen {
        std::fs::write(&golden_path, actual).unwrap();
        eprintln!("regenerated: {}", golden_path.display());
        return false;
    }
    if !golden_path.exists() {
        panic!(
            "golden file missing: {}\nRun with PANOPS_REGEN_PROMPT_GOLDENS=1 to generate",
            golden_path.display()
        );
    }
    let expected = std::fs::read_to_string(&golden_path).unwrap();
    if actual != expected {
        eprintln!(
            "prompt drift detected for '{}'\n\
             --- expected (golden)\n{}\n\
             --- actual (current)\n{}\n\
             Run with PANOPS_REGEN_PROMPT_GOLDENS=1 to re-bless",
            name, expected, actual
        );
        return true;
    }
    false
}

#[test]
fn section_narrative_prompt_goldens() {
    let regen = std::env::var("PANOPS_REGEN_PROMPT_GOLDENS").as_deref() == Ok("1");
    let goldens_dir = workspace_root().join("tests/fixtures/prompts");
    std::fs::create_dir_all(&goldens_dir).unwrap();

    let segments = sample_segments();
    let mut mismatches = 0;

    for dialect in [MarkdownDialect::NotionEnhanced, MarkdownDialect::Basic] {
        let req = build_section_narrative_prompt(&segments, dialect, "en");
        let name = format!("section_narrative_{}", dialect.as_str());
        let serialized = serialize_prompt(req.system.as_deref(), &req.user, req.schema.as_ref());
        if check_or_regenerate_golden(&name, &serialized, &goldens_dir, regen) {
            mismatches += 1;
        }
    }

    assert_eq!(mismatches, 0, "prompt regression detected");
}

#[test]
fn frontmatter_prompt_golden() {
    let regen = std::env::var("PANOPS_REGEN_PROMPT_GOLDENS").as_deref() == Ok("1");
    let goldens_dir = workspace_root().join("tests/fixtures/prompts");
    std::fs::create_dir_all(&goldens_dir).unwrap();

    let summaries = sample_summaries();
    let req = build_frontmatter_prompt(&summaries, "en", 60_000);
    let serialized = serialize_prompt(req.system.as_deref(), &req.user, req.schema.as_ref());

    let mismatch = check_or_regenerate_golden("frontmatter", &serialized, &goldens_dir, regen);
    assert!(!mismatch, "prompt regression detected");
}

#[test]
fn merge_section_prompt_goldens() {
    let regen = std::env::var("PANOPS_REGEN_PROMPT_GOLDENS").as_deref() == Ok("1");
    let goldens_dir = workspace_root().join("tests/fixtures/prompts");
    std::fs::create_dir_all(&goldens_dir).unwrap();

    let chunk_summaries = sample_chunk_summaries();
    let mut mismatches = 0;

    for dialect in [MarkdownDialect::NotionEnhanced, MarkdownDialect::Basic] {
        let req = build_merge_section_prompt(&chunk_summaries, dialect, "en");
        let name = format!("merge_section_{}", dialect.as_str());
        let serialized = serialize_prompt(req.system.as_deref(), &req.user, req.schema.as_ref());
        if check_or_regenerate_golden(&name, &serialized, &goldens_dir, regen) {
            mismatches += 1;
        }
    }

    assert_eq!(mismatches, 0, "prompt regression detected");
}
