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

use crate::storage::{MeetingDraft, MeetingListFilter, NoteDraft, Storage, StorageError};

/// Run the full conformance suite against a `Storage` implementation.
pub fn run_suite<S: Storage>(adapter: &S) {
    create_get_round_trip(adapter);
    create_duplicate_id_is_already_exists(adapter);
    create_duplicate_dir_path_is_unique_conflict(adapter);
    get_unknown_id_is_not_found(adapter);
    list_returns_started_at_desc(adapter);
    update_meeting_ended_writes_duration(adapter);
    update_meeting_language_persists(adapter);
    rename_meeting_persists_title(adapter);
    delete_meeting_cascades_to_notes(adapter);
    create_and_list_notes_for_meeting(adapter);
    replace_meeting_note_replaces_existing(adapter);
    replace_meeting_note_unknown_meeting_is_not_found(adapter);
    create_meeting_with_note_atomic_happy_path(adapter);
    create_meeting_with_note_rolls_back_on_note_collision(adapter);
    spaces_create_list_rename_delete(adapter);
    projects_create_list_rename_delete(adapter);
    tags_create_list_delete_and_idempotent_name(adapter);
    tag_meeting_untag_and_list_tags_for_meeting(adapter);
    assign_meeting_handles_space_and_project_sets_space(adapter);
    list_meetings_supports_org_filters(adapter);
    deleting_org_rows_preserves_meetings_and_clears_refs(adapter);
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

fn rename_meeting_persists_title<S: Storage>(adapter: &S) {
    let id = "m_rename";
    let _ = adapter
        .create_meeting(draft(id, "Before", "2026-05-05T10:00:00+00:00"))
        .unwrap();
    let updated = adapter
        .rename_meeting(id, "After")
        .expect("rename_meeting should succeed");
    assert_eq!(updated.title, "After");
    // Other fields unchanged.
    assert_eq!(updated.id, id);
    assert_eq!(updated.language, "auto");

    let round = adapter.get_meeting(id).unwrap();
    assert_eq!(round.title, "After");

    let err = adapter
        .rename_meeting("does-not-exist", "X")
        .expect_err("rename of unknown id should fail");
    assert!(matches!(
        err,
        StorageError::NotFound {
            kind: "meeting",
            ..
        }
    ));
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

fn replace_meeting_note_replaces_existing<S: Storage>(adapter: &S) {
    let m = "m_replace_note";
    let _ = adapter
        .create_meeting(draft(m, "Replace Note", "2026-05-05T10:00:00+00:00"))
        .unwrap();
    let original = adapter
        .create_note(NoteDraft {
            id: "n_original".into(),
            meeting_id: m.into(),
            dialect: "notion-enhanced".into(),
            content_md: "# original".into(),
            primary_path: "/tmp/m_replace_note/notes.md".into(),
        })
        .expect("create original note");

    let replaced = adapter
        .replace_meeting_note(
            m,
            NoteDraft {
                id: "n_replaced".into(),
                meeting_id: m.into(),
                dialect: "basic".into(),
                content_md: "# edited by user".into(),
                primary_path: "/tmp/m_replace_note/notes.md".into(),
            },
        )
        .expect("replace_meeting_note should succeed");
    assert_eq!(replaced.id, "n_replaced");
    assert_eq!(replaced.content_md, "# edited by user");
    assert_eq!(replaced.dialect, "basic");
    assert!(!replaced.created_at.is_empty());
    // created_at of the replacement must not be earlier than the
    // original's — the replace is a fresh insert.
    assert!(replaced.created_at >= original.created_at);

    let rows = adapter
        .list_notes_for_meeting(m)
        .expect("list after replace");
    assert_eq!(rows.len(), 1, "replace must leave exactly one note row");
    assert_eq!(rows[0].id, "n_replaced");
    assert_eq!(rows[0].content_md, "# edited by user");

    // Replace again without any pre-existing row — should create.
    adapter.delete_meeting(m).unwrap();
    let m2 = "m_replace_note_fresh";
    let _ = adapter
        .create_meeting(draft(m2, "Fresh", "2026-05-05T11:00:00+00:00"))
        .unwrap();
    let fresh = adapter
        .replace_meeting_note(
            m2,
            NoteDraft {
                id: "n_fresh".into(),
                meeting_id: m2.into(),
                dialect: "basic".into(),
                content_md: "# fresh".into(),
                primary_path: "/tmp/m_fresh/notes.md".into(),
            },
        )
        .expect("replace on meeting with no notes should create");
    assert_eq!(fresh.content_md, "# fresh");
    let rows = adapter.list_notes_for_meeting(m2).unwrap();
    assert_eq!(rows.len(), 1);
}

fn replace_meeting_note_unknown_meeting_is_not_found<S: Storage>(adapter: &S) {
    let err = adapter
        .replace_meeting_note(
            "does-not-exist",
            NoteDraft {
                id: "n_orphan".into(),
                meeting_id: "does-not-exist".into(),
                dialect: "basic".into(),
                content_md: "x".into(),
                primary_path: "/tmp/x.md".into(),
            },
        )
        .expect_err("replace for unknown meeting must fail");
    assert!(matches!(
        err,
        StorageError::NotFound {
            kind: "meeting",
            ..
        }
    ));
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

fn spaces_create_list_rename_delete<S: Storage>(adapter: &S) {
    let space = adapter.create_space("Work").expect("create space");
    assert_eq!(space.name, "Work");
    assert!(space.position >= 0);

    adapter
        .rename_space(&space.id, "Deep Work")
        .expect("rename space");
    let spaces = adapter.list_spaces().expect("list spaces");
    assert!(
        spaces
            .iter()
            .any(|s| s.id == space.id && s.name == "Deep Work")
    );

    adapter.delete_space(&space.id).expect("delete space");
    let spaces = adapter.list_spaces().expect("list spaces after delete");
    assert!(!spaces.iter().any(|s| s.id == space.id));

    let err = adapter
        .rename_space(&space.id, "Missing")
        .expect_err("renaming deleted space should fail");
    assert!(matches!(err, StorageError::NotFound { kind: "space", .. }));
}

fn projects_create_list_rename_delete<S: Storage>(adapter: &S) {
    let space_a = adapter.create_space("Projects A").expect("space a");
    let space_b = adapter.create_space("Projects B").expect("space b");
    let project = adapter
        .create_project(&space_a.id, "Launch")
        .expect("create project");
    let other = adapter
        .create_project(&space_b.id, "Other")
        .expect("create other project");
    assert_eq!(project.space_id, space_a.id);

    adapter
        .rename_project(&project.id, "Renamed Launch")
        .expect("rename project");
    let filtered = adapter
        .list_projects(Some(&space_a.id))
        .expect("list projects for one space");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, project.id);
    assert_eq!(filtered[0].name, "Renamed Launch");

    let all = adapter.list_projects(None).expect("list all projects");
    assert!(all.iter().any(|p| p.id == other.id));

    adapter.delete_project(&project.id).expect("delete project");
    let filtered = adapter
        .list_projects(Some(&space_a.id))
        .expect("list projects after delete");
    assert!(filtered.is_empty());

    let err = adapter
        .create_project("missing-space", "Nope")
        .expect_err("project in missing space should fail");
    assert!(matches!(err, StorageError::NotFound { kind: "space", .. }));
}

fn tags_create_list_delete_and_idempotent_name<S: Storage>(adapter: &S) {
    let tag = adapter.create_tag("urgent").expect("create tag");
    let same = adapter
        .create_tag("urgent")
        .expect("create tag with same name should be idempotent");
    assert_eq!(tag, same);

    let tags = adapter.list_tags().expect("list tags");
    assert!(tags.iter().any(|t| t.id == tag.id && t.name == "urgent"));

    adapter.delete_tag(&tag.id).expect("delete tag");
    let tags = adapter.list_tags().expect("list tags after delete");
    assert!(!tags.iter().any(|t| t.id == tag.id));
}

fn tag_meeting_untag_and_list_tags_for_meeting<S: Storage>(adapter: &S) {
    let meeting_id = "m_tagging";
    adapter
        .create_meeting(draft(meeting_id, "Tagging", "2026-05-09T10:00:00+00:00"))
        .expect("create meeting");
    let tag = adapter.create_tag("customer").expect("create tag");

    adapter
        .tag_meeting(meeting_id, &tag.id)
        .expect("tag meeting");
    adapter
        .tag_meeting(meeting_id, &tag.id)
        .expect("tag meeting idempotent");
    let tags = adapter
        .list_tags_for_meeting(meeting_id)
        .expect("list tags for meeting");
    assert_eq!(tags, vec![tag.clone()]);

    let summary = adapter
        .list_meetings()
        .expect("list meetings")
        .into_iter()
        .find(|m| m.id == meeting_id)
        .expect("tagged meeting summary");
    assert_eq!(summary.tags, vec![tag.id.clone()]);

    adapter
        .untag_meeting(meeting_id, &tag.id)
        .expect("untag meeting");
    let tags = adapter
        .list_tags_for_meeting(meeting_id)
        .expect("list tags after untag");
    assert!(tags.is_empty());

    let err = adapter
        .tag_meeting("missing-meeting", &tag.id)
        .expect_err("tagging missing meeting should fail");
    assert!(matches!(
        err,
        StorageError::NotFound {
            kind: "meeting",
            ..
        }
    ));
}

fn assign_meeting_handles_space_and_project_sets_space<S: Storage>(adapter: &S) {
    let meeting_id = "m_assign";
    adapter
        .create_meeting(draft(meeting_id, "Assign", "2026-05-10T10:00:00+00:00"))
        .expect("create meeting");
    let space = adapter.create_space("Assignments").expect("create space");
    let project = adapter
        .create_project(&space.id, "Nested")
        .expect("create project");

    adapter
        .assign_meeting(meeting_id, Some(space.id.clone()), None)
        .expect("assign to space");
    let space_summary = summary(adapter, meeting_id);
    assert_eq!(space_summary.space_id.as_deref(), Some(space.id.as_str()));
    assert!(space_summary.project_id.is_none());

    adapter
        .assign_meeting(meeting_id, None, Some(project.id.clone()))
        .expect("assign to project");
    let project_summary = summary(adapter, meeting_id);
    assert_eq!(project_summary.space_id.as_deref(), Some(space.id.as_str()));
    assert_eq!(
        project_summary.project_id.as_deref(),
        Some(project.id.as_str())
    );

    adapter
        .assign_meeting(meeting_id, None, None)
        .expect("clear assignment");
    let cleared_summary = summary(adapter, meeting_id);
    assert!(cleared_summary.space_id.is_none());
    assert!(cleared_summary.project_id.is_none());
}

fn list_meetings_supports_org_filters<S: Storage>(adapter: &S) {
    let inbox_id = "m_filter_inbox";
    let space_id = "m_filter_space";
    let project_id = "m_filter_project";
    adapter
        .create_meeting(draft(inbox_id, "Inbox", "2026-05-11T10:00:00+00:00"))
        .expect("create inbox meeting");
    adapter
        .create_meeting(draft(space_id, "Space", "2026-05-12T10:00:00+00:00"))
        .expect("create space meeting");
    adapter
        .create_meeting(draft(project_id, "Project", "2026-05-13T10:00:00+00:00"))
        .expect("create project meeting");
    let space = adapter.create_space("Filter Space").expect("create space");
    let project = adapter
        .create_project(&space.id, "Filter Project")
        .expect("create project");
    let tag = adapter.create_tag("filter-tag").expect("create tag");
    adapter
        .assign_meeting(space_id, Some(space.id.clone()), None)
        .expect("assign space meeting");
    adapter
        .assign_meeting(project_id, None, Some(project.id.clone()))
        .expect("assign project meeting");
    adapter
        .tag_meeting(project_id, &tag.id)
        .expect("tag project meeting");

    let inbox_rows = adapter
        .list_meetings_filtered(MeetingListFilter {
            unsorted: true,
            ..MeetingListFilter::default()
        })
        .expect("inbox filter");
    assert!(inbox_rows.iter().any(|m| m.id == inbox_id));
    assert!(
        !inbox_rows
            .iter()
            .any(|m| m.id == space_id || m.id == project_id)
    );

    let space_rows = adapter
        .list_meetings_filtered(MeetingListFilter {
            space_id: Some(space.id.clone()),
            ..MeetingListFilter::default()
        })
        .expect("space filter");
    assert!(space_rows.iter().any(|m| m.id == space_id));
    assert!(space_rows.iter().any(|m| m.id == project_id));
    assert!(!space_rows.iter().any(|m| m.id == inbox_id));

    let project_rows = adapter
        .list_meetings_filtered(MeetingListFilter {
            project_id: Some(project.id.clone()),
            ..MeetingListFilter::default()
        })
        .expect("project filter");
    assert_eq!(
        project_rows
            .iter()
            .map(|m| m.id.as_str())
            .filter(|id| [project_id, space_id, inbox_id].contains(id))
            .collect::<Vec<_>>(),
        vec![project_id]
    );

    let tag_rows = adapter
        .list_meetings_filtered(MeetingListFilter {
            tag_id: Some(tag.id.clone()),
            ..MeetingListFilter::default()
        })
        .expect("tag filter");
    assert_eq!(
        tag_rows
            .iter()
            .map(|m| m.id.as_str())
            .filter(|id| [project_id, space_id, inbox_id].contains(id))
            .collect::<Vec<_>>(),
        vec![project_id]
    );
}

fn deleting_org_rows_preserves_meetings_and_clears_refs<S: Storage>(adapter: &S) {
    let project_meeting = "m_delete_project_ref";
    let space_meeting = "m_delete_space_ref";
    adapter
        .create_meeting(draft(
            project_meeting,
            "Project Ref",
            "2026-05-14T10:00:00+00:00",
        ))
        .expect("create project ref meeting");
    adapter
        .create_meeting(draft(
            space_meeting,
            "Space Ref",
            "2026-05-15T10:00:00+00:00",
        ))
        .expect("create space ref meeting");

    let project_space = adapter.create_space("Delete Project Space").expect("space");
    let project = adapter
        .create_project(&project_space.id, "Delete Me")
        .expect("project");
    adapter
        .assign_meeting(project_meeting, None, Some(project.id.clone()))
        .expect("assign project ref");
    adapter.delete_project(&project.id).expect("delete project");
    let project_summary = summary(adapter, project_meeting);
    assert_eq!(
        project_summary.space_id.as_deref(),
        Some(project_space.id.as_str()),
        "deleting only a project should keep the containing space assignment"
    );
    assert!(project_summary.project_id.is_none());

    let space = adapter.create_space("Delete Space").expect("space");
    let project = adapter
        .create_project(&space.id, "Deleted With Space")
        .expect("project");
    adapter
        .assign_meeting(space_meeting, None, Some(project.id.clone()))
        .expect("assign space ref");
    adapter.delete_space(&space.id).expect("delete space");
    let space_summary = summary(adapter, space_meeting);
    assert!(space_summary.space_id.is_none());
    assert!(space_summary.project_id.is_none());
    assert!(
        adapter.get_meeting(project_meeting).is_ok() && adapter.get_meeting(space_meeting).is_ok(),
        "org deletes must not delete meetings"
    );
}

fn summary<S: Storage>(adapter: &S, meeting_id: &str) -> crate::storage::MeetingSummary {
    adapter
        .list_meetings()
        .expect("list meetings")
        .into_iter()
        .find(|m| m.id == meeting_id)
        .expect("meeting summary")
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
