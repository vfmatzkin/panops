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
    get_unknown_id_is_not_found(adapter);
    list_returns_started_at_desc(adapter);
    update_meeting_ended_writes_duration(adapter);
    update_meeting_language_persists(adapter);
    delete_meeting_cascades_to_notes(adapter);
    create_and_list_notes_for_meeting(adapter);
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
    let id = "m_dup";
    let _ = adapter
        .create_meeting(draft(id, "Dup", "2026-05-05T10:00:00+00:00"))
        .expect("first create should succeed");
    let err = adapter
        .create_meeting(draft(id, "Dup again", "2026-05-05T10:00:00+00:00"))
        .expect_err("second create should fail");
    match err {
        StorageError::AlreadyExists { id: got, kind } => {
            assert_eq!(got, id);
            assert_eq!(kind, "meeting");
        }
        other => panic!("expected AlreadyExists, got {other:?}"),
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

fn draft(id: &str, title: &str, started_at: &str) -> MeetingDraft {
    MeetingDraft {
        id: id.into(),
        title: title.into(),
        started_at: started_at.into(),
        language: "auto".into(),
        dir_path: format!("/tmp/{id}"),
    }
}
