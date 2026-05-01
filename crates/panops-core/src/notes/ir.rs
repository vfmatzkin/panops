use std::path::PathBuf;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::dialect::MarkdownDialect;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredNotes {
    pub schema_version: u32,
    pub frontmatter: NotesFrontmatter,
    pub sections: Vec<NotesSection>,
    pub language: String,
    pub generated_at: DateTime<Utc>,
}

impl StructuredNotes {
    pub const SCHEMA_VERSION: u32 = 1;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotesFrontmatter {
    pub title: String,
    pub date: NaiveDate,
    pub started_at: DateTime<FixedOffset>,
    pub duration_ms: u64,
    pub speakers: Vec<String>,
    pub tags: Vec<String>,
    pub template: String,
    pub dialect: MarkdownDialect,
    pub panops_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_audio: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotesSection {
    pub index: u32,
    pub title: String,
    pub time_range_ms: (u64, u64),
    pub narrative_md: String,
    pub key_points: Vec<String>,
    pub action_items: Vec<ActionItem>,
    pub screenshots: Vec<Screenshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionItem {
    pub description: String,
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screenshot {
    pub ms_since_start: u64,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};
    use std::path::PathBuf;

    fn sample() -> StructuredNotes {
        StructuredNotes {
            schema_version: StructuredNotes::SCHEMA_VERSION,
            frontmatter: NotesFrontmatter {
                title: "Test meeting".into(),
                date: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
                started_at: chrono::FixedOffset::east_opt(0)
                    .unwrap()
                    .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                    .unwrap(),
                duration_ms: 60_000,
                speakers: vec!["speaker_0".into(), "speaker_1".into()],
                tags: vec!["test".into()],
                template: "default".into(),
                dialect: crate::notes::dialect::MarkdownDialect::NotionEnhanced,
                panops_version: "0.1.0".into(),
                source_audio: Some(PathBuf::from("audio.wav")),
            },
            sections: vec![NotesSection {
                index: 1,
                title: "Intro".into(),
                time_range_ms: (0, 60_000),
                narrative_md: "Hello".into(),
                key_points: vec![],
                action_items: vec![],
                screenshots: vec![],
            }],
            language: "en".into(),
            generated_at: Utc.with_ymd_and_hms(2026, 5, 1, 10, 1, 0).unwrap(),
        }
    }

    #[test]
    fn structured_notes_round_trips_through_serde_json() {
        let n = sample();
        let s = serde_json::to_string(&n).unwrap();
        let back: StructuredNotes = serde_json::from_str(&s).unwrap();
        assert_eq!(back.schema_version, StructuredNotes::SCHEMA_VERSION);
        assert_eq!(back.frontmatter.title, "Test meeting");
        assert_eq!(back.sections.len(), 1);
        assert_eq!(back.sections[0].title, "Intro");
    }

    #[test]
    fn action_item_with_no_owner_serializes_null() {
        let a = ActionItem {
            description: "follow up".into(),
            owner: None,
            due: None,
        };
        let s = serde_json::to_string(&a).unwrap();
        assert!(s.contains("\"owner\":null"));
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(StructuredNotes::SCHEMA_VERSION, 1);
    }
}
