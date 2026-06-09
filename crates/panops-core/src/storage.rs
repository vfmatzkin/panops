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
    fn list_meetings_filtered(
        &self,
        filter: MeetingListFilter,
    ) -> Result<Vec<MeetingSummary>, StorageError>;
    fn update_meeting_ended(
        &self,
        id: &str,
        ended_at: &str,
        duration_ms: u64,
    ) -> Result<Meeting, StorageError>;
    fn rename_meeting(&self, id: &str, title: &str) -> Result<Meeting, StorageError>;
    fn delete_meeting(&self, id: &str) -> Result<(), StorageError>;
    fn create_note(&self, draft: NoteDraft) -> Result<Note, StorageError>;
    fn list_notes_for_meeting(&self, meeting_id: &str) -> Result<Vec<Note>, StorageError>;
    /// Delete all existing note rows for `meeting_id` and insert the
    /// supplied draft as the single current note. Returns the newly
    /// inserted `Note` (with its `created_at` populated by the
    /// adapter). Used by `notes.save` so a manual edit replaces the
    /// pipeline-generated note rather than appending a second row.
    fn replace_meeting_note(
        &self,
        meeting_id: &str,
        draft: NoteDraft,
    ) -> Result<Note, StorageError>;
    fn create_space(&self, name: &str) -> Result<Space, StorageError>;
    fn list_spaces(&self) -> Result<Vec<Space>, StorageError>;
    fn rename_space(&self, id: &str, name: &str) -> Result<(), StorageError>;
    fn delete_space(&self, id: &str) -> Result<(), StorageError>;
    fn create_project(&self, space_id: &str, name: &str) -> Result<Project, StorageError>;
    fn list_projects(&self, space_id: Option<&str>) -> Result<Vec<Project>, StorageError>;
    fn rename_project(&self, id: &str, name: &str) -> Result<(), StorageError>;
    fn delete_project(&self, id: &str) -> Result<(), StorageError>;
    fn create_tag(&self, name: &str) -> Result<Tag, StorageError>;
    fn list_tags(&self) -> Result<Vec<Tag>, StorageError>;
    fn delete_tag(&self, id: &str) -> Result<(), StorageError>;
    fn tag_meeting(&self, meeting_id: &str, tag_id: &str) -> Result<(), StorageError>;
    fn untag_meeting(&self, meeting_id: &str, tag_id: &str) -> Result<(), StorageError>;
    fn list_tags_for_meeting(&self, meeting_id: &str) -> Result<Vec<Tag>, StorageError>;
    fn assign_meeting(
        &self,
        meeting_id: &str,
        space_id: Option<String>,
        project_id: Option<String>,
    ) -> Result<(), StorageError>;

    /// Atomic combined insert: meeting + note + (optional) ended_at
    /// in a single transaction. Used by the CLI's `notes` flow so a
    /// note insert failure rolls back the meeting row instead of
    /// leaving a meeting-without-note in the registry. Real adapters
    /// should use `BEGIN`/`COMMIT`; the in-memory fake fakes atomicity
    /// by constructing both records up front and inserting only after
    /// both validate.
    fn create_meeting_with_note(
        &self,
        meeting: MeetingDraft,
        note: NoteDraft,
        ended_at: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<(Meeting, Note), StorageError>;
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
    pub ended_at: Option<String>,
    pub duration_ms: u64,
    pub language: String,
    pub has_notes: bool,
    pub space_id: Option<String>,
    pub project_id: Option<String>,
    /// Tag ids assigned to this meeting.
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeetingListFilter {
    pub space_id: Option<String>,
    pub project_id: Option<String>,
    pub tag_id: Option<String>,
    /// When true, return meetings in the implicit Inbox (`space_id IS NULL`).
    pub unsorted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub space_id: String,
    pub name: String,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub id: String,
    pub name: String,
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
    /// Primary-key collision: a row with this `id` already exists.
    #[error("{kind} already exists: {id}")]
    AlreadyExists { id: String, kind: &'static str },
    /// Non-PK UNIQUE constraint collision. `field` names the column
    /// (e.g., "dir_path") and `value` is the colliding value the
    /// caller tried to write. Lets the wire layer surface a precise
    /// error like "meeting.dir_path already in use" instead of
    /// misattributing the conflict to the meeting's id (which is
    /// what a blanket `AlreadyExists` mapping would do).
    #[error("{kind}.{field} already in use: {value}")]
    UniqueConflict {
        kind: &'static str,
        field: &'static str,
        value: String,
    },
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
