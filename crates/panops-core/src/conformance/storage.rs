//! Conformance harness for [`crate::storage::Storage`] adapters.
//!
//! Every Storage impl (real `RusqliteStorage`, fake `InMemoryStorage`)
//! must pass this same suite. The harness asserts the contract
//! documented on the trait:
//!
//! - `create_meeting` returns the persisted row; round-trips via
//!   `get_meeting`. A second create with the same id is `AlreadyExists`.
//! - `get_meeting` of an unknown id is `NotFound { kind: "meeting" }`.
//! - `list_meetings` returns rows in `started_at DESC` order.
//! - `update_meeting_ended` populates `ended_at` + `duration_ms` and
//!   the round-tripped row reflects the change.
//! - `update_meeting_language` mutates language; round-trip confirms.
//! - `delete_meeting` removes the row AND cascades note deletion.
//! - `create_note` round-trips via `list_notes_for_meeting`.

use crate::storage::{MeetingDraft, NoteDraft, Storage, StorageError};

/// Run the full conformance suite against a `Storage` implementation.
pub fn run_suite<S: Storage>(adapter: &S) {
    create_get_round_trip(adapter);
    create_duplicate_id_is_already_exists(adapter);
    create_duplicate_dir_path_is_unique_conflict(adapter);
    get_unknown_id_is_not_found(adapter);
    list_returns_started_at_desc(adapter);
    update_meeting_ended_writes_duration(adapter);
    update_meeting_language_persists(adapter);
    delete_meeting_cascades_to_notes(adapter);
    create_and_list_notes_for_meeting(adapter);
    create_meeting_with_note_atomic_happy_path(adapter);
    create_meeting_with_note_rolls_back_on_note_collision(adapter);
}

fn create_get_round_trip<S: Storage>(adapter: &S) {
    let id = "m_round_trip";
    let m = adapter
        .create_meeting(draft(id, "Round Trip", "2026-05-05T10:00:00+00:00"))
        .expect("create_meeting should succeed");
    assert_eq!(m.id, id);
    assert_eq!(m.title, "Round Trip");
    assert!(m.ended_at.is_none());
    assert!(m.duration_ms.is_none());

    let again = adapter
        .get_meeting(id)
        .expect("get_meeting should find the row");
    assert_eq!(again, m);
}

fn create_duplicate_id_is_already_exists<S: Storage>(adapter: &S) {
    // Same id, DIFFERENT dir_path — isolates the PK collision from
    // the dir_path UNIQUE collision (which is its own test).
    let id = "m_dup";
    let _ = adapter
        .create_meeting(MeetingDraft {
            id: id.into(),
            title: "Dup".into(),
            started_at: "2026-05-05T10:00:00+00:00".into(),
            language: "auto".into(),
            dir_path: "/tmp/m_dup_first".into(),
        })
        .expect("first create should succeed");
    let err = adapter
        .create_meeting(MeetingDraft {
            id: id.into(),
            title: "Dup again".into(),
            started_at: "2026-05-05T10:00:00+00:00".into(),
            language: "auto".into(),
            dir_path: "/tmp/m_dup_second".into(),
        })
        .expect_err("second create should fail");
    match err {
        StorageError::AlreadyExists { id: got, kind } => {
            assert_eq!(got, id);
            assert_eq!(kind, "meeting");
        }
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
}

fn create_duplicate_dir_path_is_unique_conflict<S: Storage>(adapter: &S) {
    // Two distinct meeting ids that share a `dir_path` must collide
    // on the schema's UNIQUE constraint and surface as
    // `UniqueConflict { field: "dir_path", value }`, NOT as
    // `AlreadyExists` (which would misattribute the conflict to the
    // newly-minted id).
    let dir = "/tmp/share_this_dir_dup";
    let _ = adapter
        .create_meeting(MeetingDraft {
            id: "m_dir_a".into(),
            title: "A".into(),
            started_at: "2026-05-05T10:00:00+00:00".into(),
            language: "auto".into(),
            dir_path: dir.into(),
        })
        .expect("first create should succeed");
    let err = adapter
        .create_meeting(MeetingDraft {
            id: "m_dir_b".into(),
            title: "B".into(),
            started_at: "2026-05-05T10:00:00+00:00".into(),
            language: "auto".into(),
            dir_path: dir.into(),
        })
        .expect_err("second create with same dir_path should fail");
    match err {
        StorageError::UniqueConflict { kind, field, value } => {
            assert_eq!(kind, "meeting");
            assert_eq!(field, "dir_path");
            assert_eq!(value, dir);
        }
        other => panic!("expected UniqueConflict on dir_path, got {other:?}"),
    }
}

fn get_unknown_id_is_not_found<S: Storage>(adapter: &S) {
    let err = adapter
        .get_meeting("does-not-exist")
        .expect_err("get of unknown id should fail");
    match err {
        StorageError::NotFound { id, kind } => {
            assert_eq!(id, "does-not-exist");
            assert_eq!(kind, "meeting");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

fn list_returns_started_at_desc<S: Storage>(adapter: &S) {
    let _ = adapter.create_meeting(draft("a", "A", "2026-05-01T10:00:00+00:00"));
    let _ = adapter.create_meeting(draft("b", "B", "2026-05-03T10:00:00+00:00"));
    let _ = adapter.create_meeting(draft("c", "C", "2026-05-02T10:00:00+00:00"));
    adapter
        .update_meeting_language("b", "es")
        .expect("language update should succeed");
    adapter
        .update_meeting_ended("b", "2026-05-03T10:30:00+00:00", 1_800_000)
        .expect("ended update should succeed");
    adapter
        .create_note(NoteDraft {
            id: "n_list_b".into(),
            meeting_id: "b".into(),
            dialect: "basic".into(),
            content_md: "# listed".into(),
            primary_path: "/tmp/b/notes.md".into(),
        })
        .expect("note create should succeed");

    let rows = adapter.list_meetings().expect("list should succeed");
    let our: Vec<&str> = rows
        .iter()
        .map(|r| r.id.as_str())
        .filter(|i| ["a", "b", "c"].contains(i))
        .collect();
    assert_eq!(
        our,
        vec!["b", "c", "a"],
        "list_meetings should return started_at DESC; got {our:?}"
    );

    let b = rows.iter().find(|r| r.id == "b").expect("b summary");
    assert_eq!(b.title, "B");
    assert_eq!(b.started_at, "2026-05-03T10:00:00+00:00");
    assert_eq!(b.ended_at.as_deref(), Some("2026-05-03T10:30:00+00:00"));
    assert_eq!(b.duration_ms, 1_800_000);
    assert_eq!(b.language, "es");
    assert!(b.has_notes, "summary should flag existing note row");

    let a = rows.iter().find(|r| r.id == "a").expect("a summary");
    assert!(a.ended_at.is_none());
    assert_eq!(a.duration_ms, 0);
    assert_eq!(a.language, "auto");
    assert!(!a.has_notes, "summary should be false without note rows");
}

fn update_meeting_ended_writes_duration<S: Storage>(adapter: &S) {
    let id = "m_end";
    let _ = adapter
        .create_meeting(draft(id, "End", "2026-05-05T10:00:00+00:00"))
        .unwrap();
    let updated = adapter
        .update_meeting_ended(id, "2026-05-05T11:00:00+00:00", 3_600_000)
        .expect("update_meeting_ended should succeed");
    assert_eq!(
        updated.ended_at.as_deref(),
        Some("2026-05-05T11:00:00+00:00")
    );
    assert_eq!(updated.duration_ms, Some(3_600_000));

    let round = adapter.get_meeting(id).unwrap();
    assert_eq!(round.ended_at.as_deref(), Some("2026-05-05T11:00:00+00:00"));
    assert_eq!(round.duration_ms, Some(3_600_000));
}

fn update_meeting_language_persists<S: Storage>(adapter: &S) {
    let id = "m_lang";
    let _ = adapter
        .create_meeting(draft(id, "Lang", "2026-05-05T10:00:00+00:00"))
        .unwrap();
    let updated = adapter
        .update_meeting_language(id, "es")
        .expect("update_meeting_language should succeed");
    assert_eq!(updated.language, "es");

    let round = adapter.get_meeting(id).unwrap();
    assert_eq!(round.language, "es");
}

fn delete_meeting_cascades_to_notes<S: Storage>(adapter: &S) {
    let m = "m_del";
    let n = "n_del";
    let _ = adapter
        .create_meeting(draft(m, "Del", "2026-05-05T10:00:00+00:00"))
        .unwrap();
    let _ = adapter
        .create_note(NoteDraft {
            id: n.into(),
            meeting_id: m.into(),
            dialect: "basic".into(),
            content_md: "# hi".into(),
            primary_path: "/tmp/notes.md".into(),
        })
        .unwrap();

    adapter.delete_meeting(m).expect("delete should succeed");
    let err = adapter.get_meeting(m).expect_err("meeting should be gone");
    assert!(matches!(err, StorageError::NotFound { .. }));

    // Notes for the deleted meeting must be gone too (FK cascade).
    let notes = adapter
        .list_notes_for_meeting(m)
        .expect("list notes should not error on a deleted meeting");
    assert!(
        notes.is_empty(),
        "expected zero notes after cascade, got {notes:?}"
    );
}

fn create_and_list_notes_for_meeting<S: Storage>(adapter: &S) {
    let m = "m_notes";
    let _ = adapter
        .create_meeting(draft(m, "Notes", "2026-05-05T10:00:00+00:00"))
        .unwrap();
    let n = adapter
        .create_note(NoteDraft {
            id: "n_one".into(),
            meeting_id: m.into(),
            dialect: "notion-enhanced".into(),
            content_md: "# notes".into(),
            primary_path: "/tmp/m_notes/notes.md".into(),
        })
        .expect("create_note should succeed");
    assert_eq!(n.id, "n_one");
    assert!(!n.created_at.is_empty(), "created_at should be set");

    let rows = adapter
        .list_notes_for_meeting(m)
        .expect("list_notes should succeed");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "n_one");
}

fn create_meeting_with_note_atomic_happy_path<S: Storage>(adapter: &S) {
    let m_id = "m_atomic_ok";
    let n_id = "n_atomic_ok";
    let (m, n) = adapter
        .create_meeting_with_note(
            MeetingDraft {
                id: m_id.into(),
                title: "Atomic OK".into(),
                started_at: "2026-05-05T10:00:00+00:00".into(),
                language: "auto".into(),
                dir_path: format!("/tmp/{m_id}"),
            },
            NoteDraft {
                id: n_id.into(),
                meeting_id: m_id.into(),
                dialect: "basic".into(),
                content_md: "# atomic".into(),
                primary_path: format!("/tmp/{m_id}/notes.md"),
            },
            Some("2026-05-05T10:30:00+00:00"),
            Some(1_800_000),
        )
        .expect("atomic create should succeed");
    assert_eq!(m.id, m_id);
    assert_eq!(m.ended_at.as_deref(), Some("2026-05-05T10:30:00+00:00"));
    assert_eq!(m.duration_ms, Some(1_800_000));
    assert_eq!(n.id, n_id);
    assert_eq!(n.meeting_id, m_id);

    let notes = adapter.list_notes_for_meeting(m_id).unwrap();
    assert_eq!(notes.len(), 1);
}

fn create_meeting_with_note_rolls_back_on_note_collision<S: Storage>(adapter: &S) {
    // Pre-seed a meeting + note. Then attempt to create_meeting_with_note
    // using a fresh meeting id but the existing note id — note insert
    // must fail AND the meeting insert must NOT have committed.
    let pre_m = "m_atomic_pre";
    let collide_n = "n_atomic_pre";
    adapter
        .create_meeting(MeetingDraft {
            id: pre_m.into(),
            title: "Pre".into(),
            started_at: "2026-05-05T10:00:00+00:00".into(),
            language: "auto".into(),
            dir_path: format!("/tmp/{pre_m}"),
        })
        .unwrap();
    adapter
        .create_note(NoteDraft {
            id: collide_n.into(),
            meeting_id: pre_m.into(),
            dialect: "basic".into(),
            content_md: "pre".into(),
            primary_path: "/tmp/pre".into(),
        })
        .unwrap();

    let new_m = "m_atomic_rollback";
    let err = adapter
        .create_meeting_with_note(
            MeetingDraft {
                id: new_m.into(),
                title: "Rollback".into(),
                started_at: "2026-05-05T10:00:00+00:00".into(),
                language: "auto".into(),
                dir_path: format!("/tmp/{new_m}"),
            },
            NoteDraft {
                // Collides with the pre-seeded note id.
                id: collide_n.into(),
                meeting_id: new_m.into(),
                dialect: "basic".into(),
                content_md: "should not commit".into(),
                primary_path: format!("/tmp/{new_m}/notes.md"),
            },
            None,
            None,
        )
        .expect_err("note id collision must fail the combined insert");

    assert!(
        matches!(err, StorageError::AlreadyExists { kind: "note", .. }),
        "expected AlreadyExists for the note, got {err:?}"
    );

    // The new meeting must NOT exist (rolled back).
    let look = adapter.get_meeting(new_m);
    assert!(
        matches!(look, Err(StorageError::NotFound { .. })),
        "expected new_m to be rolled back, got {look:?}"
    );
}

fn draft(id: &str, title: &str, started_at: &str) -> MeetingDraft {
    MeetingDraft {
        id: id.into(),
        title: title.into(),
        started_at: started_at.into(),
        language: "auto".into(),
        dir_path: format!("/tmp/{id}"),
    }
}
