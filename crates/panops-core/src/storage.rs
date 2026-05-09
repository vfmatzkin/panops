//! Storage port. Real impl: `panops_portable::RusqliteStorage`.
//! Fake: `panops_core::conformance::fakes::InMemoryStorage`.
//!
//! The trait is sync; async wrapping happens at the handler layer via
//! `tokio::task::spawn_blocking`. This keeps `panops-core`
//! async-runtime-free and matches the shape of the other ports
//! (`AsrProvider`, `LlmProvider`, `Diarizer`, `NotesExporter`).

use thiserror::Error;

pub trait Storage: Send + Sync {
    fn create_meeting(&self, draft: MeetingDraft) -> Result<Meeting, StorageError>;
    fn get_meeting(&self, id: &str) -> Result<Meeting, StorageError>;
    fn list_meetings(&self) -> Result<Vec<MeetingSummary>, StorageError>;
    fn update_meeting_ended(
        &self,
        id: &str,
        ended_at: &str,
        duration_ms: u64,
    ) -> Result<Meeting, StorageError>;
    fn update_meeting_language(&self, id: &str, language: &str) -> Result<Meeting, StorageError>;
    fn delete_meeting(&self, id: &str) -> Result<(), StorageError>;
    fn create_note(&self, draft: NoteDraft) -> Result<Note, StorageError>;
    fn list_notes_for_meeting(&self, meeting_id: &str) -> Result<Vec<Note>, StorageError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingDraft {
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub language: String,
    pub dir_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub language: String,
    pub dir_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingSummary {
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteDraft {
    pub id: String,
    pub meeting_id: String,
    pub dialect: String,
    pub content_md: String,
    pub primary_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: String,
    pub meeting_id: String,
    pub dialect: String,
    pub content_md: String,
    pub primary_path: String,
    pub created_at: String,
}

/// Domain error. NEVER derive `Serialize` (per AGENTS.md: domain
/// errors stay platform-agnostic; transport conversion lives in
/// `panops-protocol` behind the `domain-conversions` feature).
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("{kind} not found: {id}")]
    NotFound { id: String, kind: &'static str },
    #[error("{kind} already exists: {id}")]
    AlreadyExists { id: String, kind: &'static str },
    #[error("storage schema mismatch: actual {actual}, expected {expected}")]
    SchemaMismatch { actual: u32, expected: u32 },
    #[error("io: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },
    #[error("sql: {message}")]
    Sql { message: String },
}

impl StorageError {
    /// Construct a `Sql` variant from anything `Display`. Used by
    /// `RusqliteStorage` to keep `panops-core` rusqlite-free.
    pub fn sql<E: std::fmt::Display>(e: E) -> Self {
        Self::Sql {
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_draft_can_be_constructed() {
        let d = MeetingDraft {
            id: "m1".into(),
            title: "Test".into(),
            started_at: "2026-05-05T10:00:00+00:00".into(),
            language: "en".into(),
            dir_path: "/tmp/x".into(),
        };
        assert_eq!(d.id, "m1");
    }

    #[test]
    fn storage_error_display_includes_kind() {
        let e = StorageError::NotFound {
            id: "abc".into(),
            kind: "meeting",
        };
        let s = format!("{e}");
        assert!(s.contains("meeting"), "got: {s}");
        assert!(s.contains("abc"), "got: {s}");
    }

    #[test]
    fn storage_error_io_via_from() {
        let io: std::io::Error = std::io::Error::other("disk full");
        let e: StorageError = io.into();
        assert!(matches!(e, StorageError::Io { .. }));
    }

    #[test]
    fn storage_error_sql_helper_does_not_require_rusqlite() {
        let e = StorageError::sql("any displayable");
        assert!(matches!(e, StorageError::Sql { .. }));
    }
}
