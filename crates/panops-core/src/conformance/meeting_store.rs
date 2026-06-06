//! Conformance harness for [`crate::meeting_store::MeetingStore`] adapters.
//!
//! Every MeetingStore impl (real `RusqliteMeetingStore`, fake
//! `InMemoryMeetingStore`) must pass this same suite. The harness asserts
//! the contract documented on the trait:
//!
//! - `create_segment` returns a row with auto-generated id.
//! - `list_segments` returns segments ordered by start_ms.
//! - `create_screenshot` returns a row with auto-generated id.
//! - `list_screenshots` returns screenshots ordered by timestamp_ms.
//! - `create_speaker` returns a row with auto-generated id.
//! - `list_speakers` returns speakers for the meeting.
//! - `get_speaker` returns a speaker by id.
//! - `update_speaker_label` mutates the label.

use crate::meeting_store::{
    MeetingStore, MeetingStoreError, ScreenshotDraft, SegmentDraft, SpeakerDraft,
};

/// Run the full conformance suite against a `MeetingStore` implementation.
pub fn run_suite<M: MeetingStore>(adapter: &M) {
    create_and_list_segments(adapter);
    create_and_list_screenshots(adapter);
    create_and_list_speakers(adapter);
    get_speaker(adapter);
    update_speaker_label(adapter);
    get_nonexistent_speaker(adapter);
}

fn create_and_list_segments<M: MeetingStore>(adapter: &M) {
    let meeting_id = "test_meeting_001";

    // Create two segments.
    let s1 = adapter
        .create_segment(SegmentDraft {
            meeting_id: meeting_id.into(),
            start_ms: 0,
            end_ms: 1000,
            text: "hello world".into(),
            language: Some("en".into()),
            confidence: Some(0.9),
            speaker_id: None,
            source: "post_pass".into(),
        })
        .expect("create_segment should succeed");
    assert!(s1.id > 0, "segment id should be auto-generated");
    assert_eq!(s1.meeting_id, meeting_id);
    assert_eq!(s1.text, "hello world");

    let s2 = adapter
        .create_segment(SegmentDraft {
            meeting_id: meeting_id.into(),
            start_ms: 2000,
            end_ms: 3000,
            text: "goodbye".into(),
            language: Some("en".into()),
            confidence: Some(0.8),
            speaker_id: None,
            source: "post_pass".into(),
        })
        .expect("create_segment should succeed");

    // List segments, ordered by start_ms.
    let segments = adapter
        .list_segments(meeting_id)
        .expect("list_segments should succeed");
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].id, s1.id);
    assert_eq!(segments[1].id, s2.id);
}

fn create_and_list_screenshots<M: MeetingStore>(adapter: &M) {
    let meeting_id = "test_meeting_screenshots";

    // Create two screenshots.
    let sc1 = adapter
        .create_screenshot(ScreenshotDraft {
            meeting_id: meeting_id.into(),
            timestamp_ms: 5000,
            path: "/tmp/screenshots/001.jpg".into(),
            feature_print: None,
            caption: None,
        })
        .expect("create_screenshot should succeed");
    assert!(sc1.id > 0, "screenshot id should be auto-generated");
    assert_eq!(sc1.meeting_id, meeting_id);

    let sc2 = adapter
        .create_screenshot(ScreenshotDraft {
            meeting_id: meeting_id.into(),
            timestamp_ms: 10_000,
            path: "/tmp/screenshots/002.jpg".into(),
            feature_print: None,
            caption: None,
        })
        .expect("create_screenshot should succeed");

    // List screenshots, ordered by timestamp_ms.
    let screenshots = adapter
        .list_screenshots(meeting_id)
        .expect("list_screenshots should succeed");
    assert_eq!(screenshots.len(), 2);
    assert_eq!(screenshots[0].id, sc1.id);
    assert_eq!(screenshots[1].id, sc2.id);
}

fn create_and_list_speakers<M: MeetingStore>(adapter: &M) {
    let meeting_id = "test_meeting_speakers";

    // Create two speakers.
    let sp1 = adapter
        .create_speaker(SpeakerDraft {
            meeting_id: meeting_id.into(),
            label: "Speaker A".into(),
            embedding: None,
        })
        .expect("create_speaker should succeed");
    assert!(sp1.id > 0, "speaker id should be auto-generated");
    assert_eq!(sp1.meeting_id, meeting_id);
    assert_eq!(sp1.label, "Speaker A");

    let sp2 = adapter
        .create_speaker(SpeakerDraft {
            meeting_id: meeting_id.into(),
            label: "Speaker B".into(),
            embedding: None,
        })
        .expect("create_speaker should succeed");

    // List speakers.
    let speakers = adapter
        .list_speakers(meeting_id)
        .expect("list_speakers should succeed");
    assert_eq!(speakers.len(), 2);
    // Order is not guaranteed; check both exist.
    let ids: Vec<i64> = speakers.iter().map(|s| s.id).collect();
    assert!(ids.contains(&sp1.id));
    assert!(ids.contains(&sp2.id));
}

fn get_speaker<M: MeetingStore>(adapter: &M) {
    let meeting_id = "test_meeting_get_speaker";

    let sp = adapter
        .create_speaker(SpeakerDraft {
            meeting_id: meeting_id.into(),
            label: "Speaker X".into(),
            embedding: None,
        })
        .expect("create_speaker should succeed");

    let fetched = adapter
        .get_speaker(sp.id)
        .expect("get_speaker should succeed");
    assert_eq!(fetched.id, sp.id);
    assert_eq!(fetched.label, "Speaker X");
}

fn update_speaker_label<M: MeetingStore>(adapter: &M) {
    let meeting_id = "test_meeting_update_speaker";

    let sp = adapter
        .create_speaker(SpeakerDraft {
            meeting_id: meeting_id.into(),
            label: "Original Label".into(),
            embedding: None,
        })
        .expect("create_speaker should succeed");

    let updated = adapter
        .update_speaker_label(sp.id, "New Label")
        .expect("update_speaker_label should succeed");
    assert_eq!(updated.id, sp.id);
    assert_eq!(updated.label, "New Label");

    // Verify persisted.
    let fetched = adapter
        .get_speaker(sp.id)
        .expect("get_speaker should succeed");
    assert_eq!(fetched.label, "New Label");
}

fn get_nonexistent_speaker<M: MeetingStore>(adapter: &M) {
    let err = adapter
        .get_speaker(999999)
        .expect_err("get_speaker of nonexistent id should fail");
    match err {
        MeetingStoreError::SpeakerNotFound { id } => assert_eq!(id, 999999),
        other => panic!("expected SpeakerNotFound, got {other:?}"),
    }
}
