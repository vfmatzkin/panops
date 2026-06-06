//! `MeetingStore` real adapter backed by `rusqlite` against a per-meeting
//! SQLite file at `<meeting_dir>/meeting.db`. Stores segments, screenshots,
//! and speakers.
//!
//! One instance per meeting; each meeting has its own DB file.
//! Thread-safe via `Arc<Mutex<Connection>>`.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};

use panops_core::meeting_store::{
    MeetingStore, MeetingStoreError, ScreenshotDraft, ScreenshotRow, SegmentDraft, SegmentRow,
    SpeakerDraft, SpeakerRow,
};

/// Lock the connection mutex, mapping a poisoned mutex to a
/// `MeetingStoreError::Sql` instead of panicking.
fn lock<'a>(m: &'a Mutex<Connection>) -> Result<MutexGuard<'a, Connection>, MeetingStoreError> {
    m.lock()
        .map_err(|e| MeetingStoreError::sql(format!("meeting store mutex poisoned: {e}")))
}

const EXPECTED_SCHEMA_VERSION: u32 = 1;

const DDL: &str = r"
CREATE TABLE IF NOT EXISTS segment (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    meeting_id TEXT NOT NULL,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER NOT NULL,
    text TEXT NOT NULL,
    language TEXT,
    confidence REAL,
    speaker_id INTEGER,
    source TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_segment_meeting_id ON segment(meeting_id);
CREATE INDEX IF NOT EXISTS idx_segment_start_ms ON segment(start_ms);

CREATE TABLE IF NOT EXISTS screenshot (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    meeting_id TEXT NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    path TEXT NOT NULL,
    feature_print BLOB,
    caption TEXT
);

CREATE INDEX IF NOT EXISTS idx_screenshot_meeting_id ON screenshot(meeting_id);
CREATE INDEX IF NOT EXISTS idx_screenshot_timestamp_ms ON screenshot(timestamp_ms);

CREATE TABLE IF NOT EXISTS speaker (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    meeting_id TEXT NOT NULL,
    label TEXT NOT NULL,
    embedding BLOB
);

CREATE INDEX IF NOT EXISTS idx_speaker_meeting_id ON speaker(meeting_id);
";

pub struct RusqliteMeetingStore {
    conn: Arc<Mutex<Connection>>,
}

impl RusqliteMeetingStore {
    /// Open or create the DB at `path`. Runs DDL if the file is fresh
    /// (`user_version = 0`) or already at the expected version.
    /// Errors with `MeetingStoreError::Sql` if `user_version` doesn't match.
    pub fn new(path: &Path) -> Result<Self, MeetingStoreError> {
        let conn = Connection::open(path).map_err(MeetingStoreError::sql)?;

        let actual: u32 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get::<_, u32>(0))
            .map_err(MeetingStoreError::sql)?;

        if actual == 0 {
            conn.execute_batch(DDL).map_err(MeetingStoreError::sql)?;
            conn.execute_batch(&format!("PRAGMA user_version = {EXPECTED_SCHEMA_VERSION};"))
                .map_err(MeetingStoreError::sql)?;
        } else if actual != EXPECTED_SCHEMA_VERSION {
            return Err(MeetingStoreError::Sql {
                message: format!(
                    "meeting.db schema mismatch: expected {EXPECTED_SCHEMA_VERSION}, got {actual}"
                ),
            });
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl MeetingStore for RusqliteMeetingStore {
    fn create_segment(&self, draft: SegmentDraft) -> Result<SegmentRow, MeetingStoreError> {
        let conn = lock(&self.conn)?;
        let id = conn.query_row(
            "INSERT INTO segment (meeting_id, start_ms, end_ms, text, language, confidence, speaker_id, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             RETURNING id",
            params![
                draft.meeting_id,
                draft.start_ms as i64,
                draft.end_ms as i64,
                draft.text,
                draft.language,
                draft.confidence,
                draft.speaker_id,
                draft.source,
            ],
            |r| r.get::<_, i64>(0),
        )
        .map_err(MeetingStoreError::sql)?;

        Ok(SegmentRow {
            id,
            meeting_id: draft.meeting_id,
            start_ms: draft.start_ms,
            end_ms: draft.end_ms,
            text: draft.text,
            language: draft.language,
            confidence: draft.confidence,
            speaker_id: draft.speaker_id,
            source: draft.source,
        })
    }

    fn list_segments(&self, meeting_id: &str) -> Result<Vec<SegmentRow>, MeetingStoreError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, meeting_id, start_ms, end_ms, text, language, confidence, speaker_id, source
                 FROM segment WHERE meeting_id = ?1 ORDER BY start_ms ASC",
            )
            .map_err(MeetingStoreError::sql)?;

        let rows = stmt
            .query_map(params![meeting_id], |r| {
                Ok(SegmentRow {
                    id: r.get(0)?,
                    meeting_id: r.get(1)?,
                    // Clamp negative values to 0 (defense against DB corruption).
                    start_ms: r.get::<_, i64>(2)?.max(0) as u64,
                    end_ms: r.get::<_, i64>(3)?.max(0) as u64,
                    text: r.get(4)?,
                    language: r.get(5)?,
                    confidence: r.get(6)?,
                    speaker_id: r.get(7)?,
                    source: r.get(8)?,
                })
            })
            .map_err(MeetingStoreError::sql)?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(MeetingStoreError::sql)?);
        }
        Ok(out)
    }

    fn create_screenshot(
        &self,
        draft: ScreenshotDraft,
    ) -> Result<ScreenshotRow, MeetingStoreError> {
        let conn = lock(&self.conn)?;
        let id = conn
            .query_row(
                "INSERT INTO screenshot (meeting_id, timestamp_ms, path, feature_print, caption)
             VALUES (?1, ?2, ?3, ?4, ?5)
             RETURNING id",
                params![
                    draft.meeting_id,
                    draft.timestamp_ms as i64,
                    draft.path,
                    draft.feature_print,
                    draft.caption,
                ],
                |r| r.get::<_, i64>(0),
            )
            .map_err(MeetingStoreError::sql)?;

        Ok(ScreenshotRow {
            id,
            meeting_id: draft.meeting_id,
            timestamp_ms: draft.timestamp_ms,
            path: draft.path,
            feature_print: draft.feature_print,
            caption: draft.caption,
        })
    }

    fn list_screenshots(&self, meeting_id: &str) -> Result<Vec<ScreenshotRow>, MeetingStoreError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, meeting_id, timestamp_ms, path, feature_print, caption
                 FROM screenshot WHERE meeting_id = ?1 ORDER BY timestamp_ms ASC",
            )
            .map_err(MeetingStoreError::sql)?;

        let rows = stmt
            .query_map(params![meeting_id], |r| {
                Ok(ScreenshotRow {
                    id: r.get(0)?,
                    meeting_id: r.get(1)?,
                    timestamp_ms: r.get::<_, i64>(2)?.max(0) as u64,
                    path: r.get(3)?,
                    feature_print: r.get(4)?,
                    caption: r.get(5)?,
                })
            })
            .map_err(MeetingStoreError::sql)?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(MeetingStoreError::sql)?);
        }
        Ok(out)
    }

    fn create_speaker(&self, draft: SpeakerDraft) -> Result<SpeakerRow, MeetingStoreError> {
        let conn = lock(&self.conn)?;
        let id = conn
            .query_row(
                "INSERT INTO speaker (meeting_id, label, embedding)
             VALUES (?1, ?2, ?3)
             RETURNING id",
                params![draft.meeting_id, draft.label, draft.embedding],
                |r| r.get::<_, i64>(0),
            )
            .map_err(MeetingStoreError::sql)?;

        Ok(SpeakerRow {
            id,
            meeting_id: draft.meeting_id,
            label: draft.label,
            embedding: draft.embedding,
        })
    }

    fn list_speakers(&self, meeting_id: &str) -> Result<Vec<SpeakerRow>, MeetingStoreError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, meeting_id, label, embedding
                 FROM speaker WHERE meeting_id = ?1",
            )
            .map_err(MeetingStoreError::sql)?;

        let rows = stmt
            .query_map(params![meeting_id], |r| {
                Ok(SpeakerRow {
                    id: r.get(0)?,
                    meeting_id: r.get(1)?,
                    label: r.get(2)?,
                    embedding: r.get(3)?,
                })
            })
            .map_err(MeetingStoreError::sql)?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(MeetingStoreError::sql)?);
        }
        Ok(out)
    }

    fn get_speaker(&self, id: i64) -> Result<SpeakerRow, MeetingStoreError> {
        let conn = lock(&self.conn)?;
        conn.query_row(
            "SELECT id, meeting_id, label, embedding FROM speaker WHERE id = ?1",
            params![id],
            |r| {
                Ok(SpeakerRow {
                    id: r.get(0)?,
                    meeting_id: r.get(1)?,
                    label: r.get(2)?,
                    embedding: r.get(3)?,
                })
            },
        )
        .optional()
        .map_err(MeetingStoreError::sql)?
        .ok_or(MeetingStoreError::SpeakerNotFound { id })
    }

    fn update_speaker_label(&self, id: i64, label: &str) -> Result<SpeakerRow, MeetingStoreError> {
        let conn = lock(&self.conn)?;
        let n = conn
            .execute(
                "UPDATE speaker SET label = ?2 WHERE id = ?1",
                params![id, label],
            )
            .map_err(MeetingStoreError::sql)?;
        if n == 0 {
            return Err(MeetingStoreError::SpeakerNotFound { id });
        }
        drop(conn);
        self.get_speaker(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panops_core::conformance::meeting_store::run_suite;

    #[test]
    fn rusqlite_meeting_store_passes_conformance() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("meeting.db");
        let store = RusqliteMeetingStore::new(&db).unwrap();
        run_suite(&store);
    }
}
