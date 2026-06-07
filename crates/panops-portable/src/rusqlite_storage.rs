//! `Storage` real adapter backed by `rusqlite` against a single
//! SQLite file. Single-user local-first; one
//! `Arc<Mutex<Connection>>` serializes all access. Per-meeting DB at
//! `meetings/<uuid>/meeting.db` is deferred to Anchor B (live
//! capture); for now everything lives in one registry DB.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use panops_core::storage::{
    Meeting, MeetingDraft, MeetingSummary, Note, NoteDraft, Storage, StorageError,
};

/// Lock the connection mutex, mapping a poisoned mutex to a
/// `StorageError::Sql` instead of panicking. A poisoned mutex
/// indicates a panic happened in another storage call (rare, since
/// our calls are short rusqlite operations) — surfacing it as a
/// recoverable storage error is friendlier to long-running callers
/// than tearing down the whole server with another panic.
fn lock<'a>(m: &'a Mutex<Connection>) -> Result<MutexGuard<'a, Connection>, StorageError> {
    m.lock().map_err(|e| StorageError::Sql {
        message: format!("storage mutex poisoned: {e}"),
    })
}

/// Inspect a rusqlite SqliteFailure on `meeting` insert and choose
/// the right `StorageError` variant. PK collision (`meeting.id`) maps
/// to `AlreadyExists`; the secondary `dir_path` UNIQUE collision maps
/// to `UniqueConflict { field: "dir_path", value }` so callers don't
/// receive a misleading id-already-exists error when the actual
/// conflict was on a different column.
fn map_meeting_constraint_violation(
    err: rusqlite::Error,
    draft_id: &str,
    draft_dir_path: &str,
) -> StorageError {
    if let rusqlite::Error::SqliteFailure(_, Some(ref msg)) = err {
        // libsqlite encodes the constraint name in the message:
        //   "UNIQUE constraint failed: meeting.id"
        //   "UNIQUE constraint failed: meeting.dir_path"
        if msg.contains("meeting.dir_path") {
            return StorageError::UniqueConflict {
                kind: "meeting",
                field: "dir_path",
                value: draft_dir_path.to_string(),
            };
        }
        if msg.contains("meeting.id") {
            return StorageError::AlreadyExists {
                id: draft_id.to_string(),
                kind: "meeting",
            };
        }
    }
    StorageError::sql(err)
}

const EXPECTED_SCHEMA_VERSION: u32 = 1;

// `PRAGMA foreign_keys = ON` is set per-connection in `new()`, not in DDL —
// the DDL `PRAGMA` would be redundant on a fresh DB and a no-op on re-open
// (PRAGMAs aren't persisted in SQLite; they're connection-scoped).
const DDL: &str = r"
CREATE TABLE IF NOT EXISTS meeting (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL,
    ended_at TEXT,
    duration_ms INTEGER,
    language TEXT NOT NULL DEFAULT 'auto',
    dir_path TEXT NOT NULL UNIQUE
);

CREATE INDEX IF NOT EXISTS idx_meeting_started_at ON meeting(started_at DESC);

CREATE TABLE IF NOT EXISTS note (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meeting(id) ON DELETE CASCADE,
    dialect TEXT NOT NULL,
    content_md TEXT NOT NULL,
    primary_path TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_note_meeting_id ON note(meeting_id);
";

pub struct RusqliteStorage {
    conn: Arc<Mutex<Connection>>,
}

impl RusqliteStorage {
    /// Open or create the DB at `path`. Runs DDL if the file is fresh
    /// (`user_version = 0`) or already at the expected version.
    /// Errors with `SchemaMismatch` if `user_version` is set but
    /// doesn't match `EXPECTED_SCHEMA_VERSION`.
    pub fn new(path: &Path) -> Result<Self, StorageError> {
        let conn = Connection::open(path).map_err(StorageError::sql)?;
        // Always enable FKs per-connection (rusqlite default is OFF).
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(StorageError::sql)?;

        let actual: u32 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get::<_, u32>(0))
            .map_err(StorageError::sql)?;

        if actual == 0 {
            conn.execute_batch(DDL).map_err(StorageError::sql)?;
            conn.execute_batch(&format!("PRAGMA user_version = {EXPECTED_SCHEMA_VERSION};"))
                .map_err(StorageError::sql)?;
        } else if actual != EXPECTED_SCHEMA_VERSION {
            return Err(StorageError::SchemaMismatch {
                actual,
                expected: EXPECTED_SCHEMA_VERSION,
            });
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

fn map_meeting_row(r: &rusqlite::Row) -> rusqlite::Result<Meeting> {
    Ok(Meeting {
        id: r.get(0)?,
        title: r.get(1)?,
        started_at: r.get(2)?,
        ended_at: r.get(3)?,
        // Clamp negative durations to 0. SQLite stores `INTEGER` as
        // signed i64; a negative value (corruption, manual edit) cast
        // straight to u64 would wrap to ~9 quintillion ms and
        // surface as a nonsensical wire value. Clamping is the
        // conservative read-side defense; insert-side validation
        // would also reject negatives before storage but read-side
        // doesn't trust the database file.
        duration_ms: r
            .get::<_, Option<i64>>(4)?
            .map(|v| if v < 0 { 0 } else { v as u64 }),
        language: r.get(5)?,
        dir_path: r.get(6)?,
    })
}

impl Storage for RusqliteStorage {
    fn create_meeting(&self, d: MeetingDraft) -> Result<Meeting, StorageError> {
        let conn = lock(&self.conn)?;
        let result = conn.execute(
            "INSERT INTO meeting (id, title, started_at, language, dir_path)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![d.id, d.title, d.started_at, d.language, d.dir_path],
        );
        match result {
            Ok(_) => Ok(Meeting {
                id: d.id,
                title: d.title,
                started_at: d.started_at,
                ended_at: None,
                duration_ms: None,
                language: d.language,
                dir_path: d.dir_path,
            }),
            Err(rusqlite::Error::SqliteFailure(e, ref msg))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(map_meeting_constraint_violation(
                    rusqlite::Error::SqliteFailure(e, msg.clone()),
                    &d.id,
                    &d.dir_path,
                ))
            }
            Err(e) => Err(StorageError::sql(e)),
        }
    }

    fn create_meeting_with_note(
        &self,
        meeting: MeetingDraft,
        note: NoteDraft,
        ended_at: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<(Meeting, Note), StorageError> {
        if note.meeting_id != meeting.id {
            return Err(StorageError::Sql {
                message: "create_meeting_with_note: note.meeting_id must match meeting.id".into(),
            });
        }
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction().map_err(StorageError::sql)?;

        // Meeting insert (with optional ended_at + duration_ms in one shot).
        let dur_i64 = duration_ms.map(|v| v as i64);
        let result = tx.execute(
            "INSERT INTO meeting
                (id, title, started_at, ended_at, duration_ms, language, dir_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                meeting.id,
                meeting.title,
                meeting.started_at,
                ended_at,
                dur_i64,
                meeting.language,
                meeting.dir_path
            ],
        );
        if let Err(e) = result {
            return Err(match e {
                rusqlite::Error::SqliteFailure(c, ref m)
                    if c.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    map_meeting_constraint_violation(
                        rusqlite::Error::SqliteFailure(c, m.clone()),
                        &meeting.id,
                        &meeting.dir_path,
                    )
                }
                other => StorageError::sql(other),
            });
        }

        // Note insert. FK on `meeting_id` references the row we just
        // inserted in this transaction; it'll resolve before COMMIT.
        let created_at = Utc::now().to_rfc3339();
        let note_result = tx.execute(
            "INSERT INTO note (id, meeting_id, dialect, content_md, primary_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                note.id,
                note.meeting_id,
                note.dialect,
                note.content_md,
                note.primary_path,
                created_at
            ],
        );
        if let Err(e) = note_result {
            // Transaction will roll back on drop (we don't COMMIT).
            return Err(match e {
                rusqlite::Error::SqliteFailure(c, _)
                    if c.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    StorageError::AlreadyExists {
                        id: note.id,
                        kind: "note",
                    }
                }
                other => StorageError::sql(other),
            });
        }

        tx.commit().map_err(StorageError::sql)?;

        Ok((
            Meeting {
                id: meeting.id,
                title: meeting.title,
                started_at: meeting.started_at,
                ended_at: ended_at.map(str::to_owned),
                duration_ms,
                language: meeting.language,
                dir_path: meeting.dir_path,
            },
            Note {
                id: note.id,
                meeting_id: note.meeting_id,
                dialect: note.dialect,
                content_md: note.content_md,
                primary_path: note.primary_path,
                created_at,
            },
        ))
    }

    fn get_meeting(&self, id: &str) -> Result<Meeting, StorageError> {
        let conn = lock(&self.conn)?;
        conn.query_row(
            "SELECT id, title, started_at, ended_at, duration_ms, language, dir_path
             FROM meeting WHERE id = ?1",
            params![id],
            map_meeting_row,
        )
        .optional()
        .map_err(StorageError::sql)?
        .ok_or_else(|| StorageError::NotFound {
            id: id.into(),
            kind: "meeting",
        })
    }

    fn list_meetings(&self) -> Result<Vec<MeetingSummary>, StorageError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn
            .prepare(
                "SELECT
                     id,
                     title,
                     started_at,
                     ended_at,
                     COALESCE(duration_ms, 0),
                     language,
                     EXISTS (SELECT 1 FROM note WHERE note.meeting_id = meeting.id)
                 FROM meeting ORDER BY started_at DESC",
            )
            .map_err(StorageError::sql)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(MeetingSummary {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    started_at: r.get(2)?,
                    ended_at: r.get(3)?,
                    // Clamp negative durations to 0 (same defense as
                    // `map_meeting_row` — the DB file is not trusted).
                    duration_ms: r.get::<_, i64>(4)?.max(0) as u64,
                    language: r.get(5)?,
                    has_notes: r.get(6)?,
                })
            })
            .map_err(StorageError::sql)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(StorageError::sql)?);
        }
        Ok(out)
    }

    fn update_meeting_ended(
        &self,
        id: &str,
        ended_at: &str,
        duration_ms: u64,
    ) -> Result<Meeting, StorageError> {
        let conn = lock(&self.conn)?;
        let n = conn
            .execute(
                "UPDATE meeting SET ended_at = ?2, duration_ms = ?3 WHERE id = ?1",
                params![id, ended_at, duration_ms as i64],
            )
            .map_err(StorageError::sql)?;
        if n == 0 {
            return Err(StorageError::NotFound {
                id: id.into(),
                kind: "meeting",
            });
        }
        drop(conn);
        self.get_meeting(id)
    }

    fn update_meeting_language(&self, id: &str, language: &str) -> Result<Meeting, StorageError> {
        let conn = lock(&self.conn)?;
        let n = conn
            .execute(
                "UPDATE meeting SET language = ?2 WHERE id = ?1",
                params![id, language],
            )
            .map_err(StorageError::sql)?;
        if n == 0 {
            return Err(StorageError::NotFound {
                id: id.into(),
                kind: "meeting",
            });
        }
        drop(conn);
        self.get_meeting(id)
    }

    fn delete_meeting(&self, id: &str) -> Result<(), StorageError> {
        let conn = lock(&self.conn)?;
        let n = conn
            .execute("DELETE FROM meeting WHERE id = ?1", params![id])
            .map_err(StorageError::sql)?;
        if n == 0 {
            return Err(StorageError::NotFound {
                id: id.into(),
                kind: "meeting",
            });
        }
        Ok(())
    }

    fn create_note(&self, d: NoteDraft) -> Result<Note, StorageError> {
        let conn = lock(&self.conn)?;
        // FK guard: surface a friendly NotFound instead of a raw FK error.
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM meeting WHERE id = ?1",
                params![d.meeting_id],
                |_| Ok(true),
            )
            .optional()
            .map_err(StorageError::sql)?
            .unwrap_or(false);
        if !exists {
            return Err(StorageError::NotFound {
                id: d.meeting_id,
                kind: "meeting",
            });
        }

        let created_at = Utc::now().to_rfc3339();
        let result = conn.execute(
            "INSERT INTO note (id, meeting_id, dialect, content_md, primary_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                d.id,
                d.meeting_id,
                d.dialect,
                d.content_md,
                d.primary_path,
                created_at
            ],
        );
        match result {
            Ok(_) => Ok(Note {
                id: d.id,
                meeting_id: d.meeting_id,
                dialect: d.dialect,
                content_md: d.content_md,
                primary_path: d.primary_path,
                created_at,
            }),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(StorageError::AlreadyExists {
                    id: d.id,
                    kind: "note",
                })
            }
            Err(e) => Err(StorageError::sql(e)),
        }
    }

    fn list_notes_for_meeting(&self, meeting_id: &str) -> Result<Vec<Note>, StorageError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, meeting_id, dialect, content_md, primary_path, created_at
                 FROM note WHERE meeting_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(StorageError::sql)?;
        let rows = stmt
            .query_map(params![meeting_id], |r| {
                Ok(Note {
                    id: r.get(0)?,
                    meeting_id: r.get(1)?,
                    dialect: r.get(2)?,
                    content_md: r.get(3)?,
                    primary_path: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })
            .map_err(StorageError::sql)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(StorageError::sql)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_db_initialises_schema_and_sets_version() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("panops.db");
        let _ = RusqliteStorage::new(&db).expect("open fresh");
        let conn = Connection::open(&db).unwrap();
        let v: u32 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, EXPECTED_SCHEMA_VERSION);
    }

    #[test]
    fn second_open_does_not_re_run_ddl() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("panops.db");
        let _ = RusqliteStorage::new(&db).unwrap();
        // Insert a row, then re-open and confirm it survived.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute(
                "INSERT INTO meeting (id, title, started_at, dir_path)
                 VALUES ('a', 'A', '2026-05-05T10:00:00+00:00', '/tmp/a')",
                [],
            )
            .unwrap();
        }
        let storage = RusqliteStorage::new(&db).unwrap();
        let m = storage.get_meeting("a").unwrap();
        assert_eq!(m.id, "a");
    }
}
