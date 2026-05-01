use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::Segment;

use super::ir::Screenshot;

/// All inputs `NotesGenerator::generate` needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotesInput {
    pub transcript: Vec<Segment>,
    pub screenshots: Vec<Screenshot>,
    pub meeting_metadata: MeetingMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingMetadata {
    pub started_at: DateTime<FixedOffset>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_hint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};
    use std::path::PathBuf;

    #[test]
    fn meeting_metadata_round_trips() {
        let m = MeetingMetadata {
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 60_000,
            source_path: Some(PathBuf::from("audio.wav")),
            language_hint: Some("en".into()),
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: MeetingMetadata = serde_json::from_str(&s).unwrap();
        assert_eq!(back.duration_ms, 60_000);
    }

    #[test]
    fn notes_input_omits_optional_fields_when_none() {
        let m = MeetingMetadata {
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 60_000,
            source_path: None,
            language_hint: None,
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(!s.contains("source_path"));
        assert!(!s.contains("language_hint"));
    }
}
