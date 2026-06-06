//! Per-meeting content storage port. Stores segments, screenshots, and speakers
//! in a per-meeting SQLite database at `meetings/<uuid>/meeting.db`.
//!
//! Real impl: `panops_portable::RusqliteMeetingStore`.
//! Fake: `panops_core::conformance::fakes::InMemoryMeetingStore`.
//!
//! This port is distinct from the cross-meeting registry `Storage` trait:
//! - `Storage` = meeting lifecycle + note registry (`panops.db`)
//! - `MeetingStore` = segment + screenshot + speaker content (`meeting.db`)
//!
//! The trait is sync; async wrapping happens at the handler layer via
//! `tokio::task::spawn_blocking`. Matches the shape of the other ports.

use thiserror::Error;

/// A transcribed segment from ASR/post-pass. Stored in `meeting.db`.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentRow {
    pub id: i64,
    pub meeting_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub language: Option<String>,
    pub confidence: Option<f32>,
    pub speaker_id: Option<i64>,
    /// Source of the segment: "post_pass" or "live".
    pub source: String,
}

/// Draft for inserting a new segment.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentDraft {
    pub meeting_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub language: Option<String>,
    pub confidence: Option<f32>,
    pub speaker_id: Option<i64>,
    pub source: String,
}

/// A captured screenshot. Stored in `meeting.db`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotRow {
    pub id: i64,
    pub meeting_id: String,
    pub timestamp_ms: u64,
    pub path: String,
    /// Vision FeaturePrint blob for dedup/search.
    pub feature_print: Option<Vec<u8>>,
    /// LLM-generated caption (future slice).
    pub caption: Option<String>,
}

/// Draft for inserting a new screenshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotDraft {
    pub meeting_id: String,
    pub timestamp_ms: u64,
    pub path: String,
    pub feature_print: Option<Vec<u8>>,
    pub caption: Option<String>,
}

/// A identified speaker. Stored in `meeting.db`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerRow {
    pub id: i64,
    pub meeting_id: String,
    pub label: String,
    /// Speaker embedding vector (future slice).
    pub embedding: Option<Vec<u8>>,
}

/// Draft for inserting a new speaker.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerDraft {
    pub meeting_id: String,
    pub label: String,
    pub embedding: Option<Vec<u8>>,
}

/// Domain error for meeting store operations. NEVER derive `Serialize`
/// (per AGENTS.md: domain errors stay platform-agnostic; transport conversion
/// lives in `panops-protocol` behind the `domain-conversions` feature).
#[derive(Debug, Error)]
pub enum MeetingStoreError {
    #[error("meeting not found: {meeting_id}")]
    MeetingNotFound { meeting_id: String },
    #[error("segment not found: {id}")]
    SegmentNotFound { id: i64 },
    #[error("screenshot not found: {id}")]
    ScreenshotNotFound { id: i64 },
    #[error("speaker not found: {id}")]
    SpeakerNotFound { id: i64 },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sql error: {message}")]
    Sql { message: String },
}

impl MeetingStoreError {
    /// Construct a `Sql` variant from anything `Display`. Used by
    /// `RusqliteMeetingStore` to keep `panops-core` rusqlite-free.
    pub fn sql<E: std::fmt::Display>(e: E) -> Self {
        Self::Sql {
            message: e.to_string(),
        }
    }
}

/// Per-meeting content storage trait. One instance per meeting;
/// each meeting has its own SQLite DB at `<meeting_dir>/meeting.db`.
pub trait MeetingStore: Send + Sync {
    /// Insert a segment row. Returns the inserted row with its auto-generated id.
    fn create_segment(&self, draft: SegmentDraft) -> Result<SegmentRow, MeetingStoreError>;

    /// List all segments for a meeting, ordered by start_ms.
    fn list_segments(&self, meeting_id: &str) -> Result<Vec<SegmentRow>, MeetingStoreError>;

    /// Insert a screenshot row. Returns the inserted row with its auto-generated id.
    fn create_screenshot(&self, draft: ScreenshotDraft)
    -> Result<ScreenshotRow, MeetingStoreError>;

    /// List all screenshots for a meeting, ordered by timestamp_ms.
    fn list_screenshots(&self, meeting_id: &str) -> Result<Vec<ScreenshotRow>, MeetingStoreError>;

    /// Insert a speaker row. Returns the inserted row with its auto-generated id.
    fn create_speaker(&self, draft: SpeakerDraft) -> Result<SpeakerRow, MeetingStoreError>;

    /// List all speakers for a meeting.
    fn list_speakers(&self, meeting_id: &str) -> Result<Vec<SpeakerRow>, MeetingStoreError>;

    /// Get a speaker by id.
    fn get_speaker(&self, id: i64) -> Result<SpeakerRow, MeetingStoreError>;

    /// Update a speaker's label.
    fn update_speaker_label(&self, id: i64, label: &str) -> Result<SpeakerRow, MeetingStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_draft_can_be_constructed() {
        let d = SegmentDraft {
            meeting_id: "m1".into(),
            start_ms: 0,
            end_ms: 1000,
            text: "hello".into(),
            language: Some("en".into()),
            confidence: Some(0.9),
            speaker_id: None,
            source: "post_pass".into(),
        };
        assert_eq!(d.meeting_id, "m1");
        assert_eq!(d.text, "hello");
    }

    #[test]
    fn screenshot_draft_can_be_constructed() {
        let d = ScreenshotDraft {
            meeting_id: "m1".into(),
            timestamp_ms: 5000,
            path: "/tmp/screenshots/001.jpg".into(),
            feature_print: None,
            caption: None,
        };
        assert_eq!(d.meeting_id, "m1");
        assert_eq!(d.timestamp_ms, 5000);
    }

    #[test]
    fn speaker_draft_can_be_constructed() {
        let d = SpeakerDraft {
            meeting_id: "m1".into(),
            label: "Speaker A".into(),
            embedding: None,
        };
        assert_eq!(d.meeting_id, "m1");
        assert_eq!(d.label, "Speaker A");
    }

    #[test]
    fn meeting_store_error_display_includes_context() {
        let e = MeetingStoreError::MeetingNotFound {
            meeting_id: "abc".into(),
        };
        assert!(format!("{e}").contains("abc"));
        let e = MeetingStoreError::SegmentNotFound { id: 42 };
        assert!(format!("{e}").contains("42"));
    }

    #[test]
    fn meeting_store_error_io_via_from() {
        let io = std::io::Error::other("disk full");
        let e: MeetingStoreError = io.into();
        assert!(matches!(e, MeetingStoreError::Io(..)));
    }
}
