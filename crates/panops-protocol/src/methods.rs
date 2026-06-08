//! JSON-RPC method params/results and WebSocket event payloads.
//!
//! Method names appear with an `ipc.` namespace at the wire level
//! (jsonrpsee `#[rpc(namespace = "ipc")]`). Param/result types are pure
//! data — no method routing happens in this crate.

use serde::{Deserialize, Deserializer, Serialize};

/// Type-tagged so the same `events` subscription multiplexes job lifecycle.
/// Future event kinds extend this enum; clients running an older
/// `panops-protocol` deserialise the new tag as `Event::Unknown(<original
/// JSON>)` and keep the subscription alive.
///
/// The `Deserialize` impl is hand-written because serde's `#[serde(other)]`
/// only accepts unit variants — it can't represent a tuple variant that
/// captures the unrecognised payload. The `Serialize` derive does the
/// usual internally-tagged shape for the typed variants and emits the raw
/// `Value` verbatim for `Unknown`.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    #[serde(rename = "job.done")]
    JobDone(JobDoneEvent),
    #[serde(rename = "job.error")]
    JobError(JobErrorEvent),
    #[serde(rename = "job.progress")]
    JobProgress(JobProgressEvent),
    /// Screenshot captured during recording (slice 11).
    #[serde(rename = "screenshot")]
    Screenshot(ScreenshotEvent),
    /// Recording progress update (slice 11).
    #[serde(rename = "recording.progress")]
    RecordingProgress(RecordingProgressEvent),
    /// Forward-compat fallback: a future engine emits an event type this
    /// build doesn't know about. The original JSON object is kept so the
    /// caller can still inspect it (e.g. log + skip) without tearing down
    /// the subscription.
    #[serde(untagged)]
    Unknown(serde_json::Value),
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let type_str = value
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde::de::Error::missing_field("type"))?;
        match type_str {
            "job.done" => serde_json::from_value::<JobDoneEvent>(value)
                .map(Event::JobDone)
                .map_err(serde::de::Error::custom),
            "job.error" => serde_json::from_value::<JobErrorEvent>(value)
                .map(Event::JobError)
                .map_err(serde::de::Error::custom),
            "job.progress" => serde_json::from_value::<JobProgressEvent>(value)
                .map(Event::JobProgress)
                .map_err(serde::de::Error::custom),
            "screenshot" => serde_json::from_value::<ScreenshotEvent>(value)
                .map(Event::Screenshot)
                .map_err(serde::de::Error::custom),
            "recording.progress" => serde_json::from_value::<RecordingProgressEvent>(value)
                .map(Event::RecordingProgress)
                .map_err(serde::de::Error::custom),
            _ => Ok(Event::Unknown(value)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobDoneEvent {
    pub job_id: String,
    pub result: NotesGenerateResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobErrorEvent {
    pub job_id: String,
    pub error: crate::IpcError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobProgressEvent {
    pub job_id: String,
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Screenshot captured during a recording session. Emitted via WebSocket
/// each time the capture sidecar detects a screen change and writes a JPEG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenshotEvent {
    pub meeting_id: String,
    pub timestamp_ms: u64,
    pub path: String,
}

/// Recording progress update. Emitted periodically during active capture
/// to inform clients of audio bytes captured and elapsed duration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordingProgressEvent {
    pub meeting_id: String,
    pub bytes_captured: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobAccepted {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmInfo {
    pub provider: String,
    pub model: String,
    pub local: bool,
}

impl LlmInfo {
    pub fn local_ollama() -> Self {
        Self {
            provider: "ollama".into(),
            model: "gemma3:4b".into(),
            local: true,
        }
    }

    pub fn apple_foundation() -> Self {
        Self {
            provider: "apple-foundation".into(),
            model: "on-device".into(),
            local: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerInfo {
    pub llm: LlmInfo,
}

/// Params for `ipc.notes.generate`.
///
/// Param structs intentionally do NOT carry `#[serde(deny_unknown_fields)]`
/// so a future engine adding a new optional knob doesn't break older
/// clients — same forward-compat philosophy as `IpcError::Unknown`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotesGenerateParams {
    pub audio: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<NotesDialect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_diarize: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// When `Some`, attach the generated note to the existing meeting.
    /// When `None`, the handler auto-creates a meeting, returns its
    /// id, and writes notes into `<data_dir>/meetings/<id>/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NotesDialect {
    NotionEnhanced,
    Basic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotesGenerateResult {
    pub primary_file: String,
    pub assets: Vec<String>,
    /// The meeting this note belongs to. Always set after slice 06
    /// (auto-created when `NotesGenerateParams.meeting_id` was `None`).
    pub meeting_id: String,
    /// Absolute path to the human-readable raw `transcript.txt` sidecar,
    /// when it was written. `None` if the best-effort sidecar write failed
    /// (the rest of the result is still valid). Optional + skip-when-none
    /// keeps the field forward-compatible for clients built against an
    /// older snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_txt_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetingSummary {
    pub id: String,
    pub title: String,
    /// RFC3339 timestamp. Kept as `String` (not `chrono::DateTime`) so this
    /// crate stays free of date-time deps; non-Rust consumers don't need
    /// a Rust-specific time crate to consume it.
    pub started_at: String,
    /// RFC3339 when the meeting has ended; `None` for in-progress meetings.
    pub ended_at: Option<String>,
    pub duration_ms: u64,
    /// BCP-47 language hint, or "auto".
    pub language: String,
    /// Whether at least one note row exists for this meeting.
    pub has_notes: bool,
}

/// Full meeting record returned by `meeting.get` / `meeting.start` /
/// `meeting.stop`. New in slice 06; `MeetingSummary` (lighter shape
/// for `meeting.list`) remains unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    /// RFC3339. See `MeetingSummary` doc on string-vs-DateTime choice.
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: Option<u64>,
    /// BCP-47 language hint, or "auto".
    pub language: String,
    /// Absolute path to the meeting directory (where notes / future
    /// audio + screenshots live).
    pub dir_path: String,
}

#[cfg(feature = "domain-conversions")]
impl From<panops_core::storage::MeetingSummary> for MeetingSummary {
    fn from(value: panops_core::storage::MeetingSummary) -> Self {
        Self {
            id: value.id,
            title: value.title,
            started_at: value.started_at,
            ended_at: value.ended_at,
            duration_ms: value.duration_ms,
            language: value.language,
            has_notes: value.has_notes,
        }
    }
}

/// Input shape for `meeting.start`. Both fields optional; server
/// applies defaults (title="", language="auto").
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MeetingConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Params for `ipc.meeting.deleteVideo`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeetingDeleteVideoParams {
    pub meeting_id: String,
}

/// Result of `ipc.meeting.deleteVideo`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeetingDeleteVideoResult {
    pub deleted: bool,
    pub freed_bytes: u64,
}

// === Recording IPC types (slice 11) ===

/// Audio source selection for `recording.start`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioSourcesWire {
    SystemOnly,
    MicOnly,
    #[default]
    SystemAndMic,
}

fn default_screenshot_interval() -> u64 {
    500
}

fn default_screenshot_threshold() -> f32 {
    0.15
}

/// Params for `ipc.recording.start`. Starts a live capture session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordingStartParams {
    pub meeting_id: String,
    /// Audio sources to capture. Defaults to SystemAndMic.
    #[serde(default)]
    pub audio_sources: AudioSourcesWire,
    /// Whether to record the screen video to `<meeting_dir>/recording.mov`.
    /// Defaults false for backwards compatibility with clients built before
    /// the video-recording toggle landed.
    #[serde(default)]
    pub record_video: bool,
    #[serde(default = "default_screenshot_interval")]
    pub screenshot_interval_ms: u64,
    #[serde(default = "default_screenshot_threshold")]
    pub screenshot_threshold: f32,
}

#[cfg(feature = "domain-conversions")]
impl From<AudioSourcesWire> for panops_core::capture::AudioSources {
    fn from(value: AudioSourcesWire) -> Self {
        match value {
            AudioSourcesWire::SystemOnly => Self::SystemOnly,
            AudioSourcesWire::MicOnly => Self::MicOnly,
            AudioSourcesWire::SystemAndMic => Self::SystemAndMic,
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<panops_core::capture::AudioSources> for AudioSourcesWire {
    fn from(value: panops_core::capture::AudioSources) -> Self {
        match value {
            panops_core::capture::AudioSources::SystemOnly => Self::SystemOnly,
            panops_core::capture::AudioSources::MicOnly => Self::MicOnly,
            panops_core::capture::AudioSources::SystemAndMic => Self::SystemAndMic,
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<&RecordingStartParams> for panops_core::capture::CaptureConfig {
    fn from(value: &RecordingStartParams) -> Self {
        Self {
            audio_sources: value.audio_sources.into(),
            record_video: value.record_video,
            screenshot_interval_ms: value.screenshot_interval_ms,
            screenshot_threshold: value.screenshot_threshold,
        }
    }
}

/// Result of `ipc.recording.start`. Confirms the recording session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordingAccepted {
    pub recording_id: String,
}

/// Params for `ipc.recording.stop`. Stops the active recording.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordingStopParams {
    pub recording_id: String,
}

/// Result of `ipc.recording.stop`. Returns paths to captured artifacts.
/// Two audio tracks (slice 11): each is non-null exactly when its source
/// was requested.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordingStopped {
    pub system_audio_path: Option<String>,
    pub mic_audio_path: Option<String>,
    pub screenshot_paths: Vec<String>,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_info_round_trips_with_llm_info() {
        let info = ServerInfo {
            llm: LlmInfo::local_ollama(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert_eq!(
            json,
            r#"{"llm":{"provider":"ollama","model":"gemma3:4b","local":true}}"#
        );
        let back: ServerInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, info);
    }

    #[test]
    fn notes_generate_params_minimal_round_trip() {
        let p = NotesGenerateParams {
            audio: "/tmp/x.wav".into(),
            dialect: None,
            llm_provider: None,
            llm_model: None,
            no_diarize: None,
            language: None,
            meeting_id: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        // Optional fields with skip_serializing_if must be absent.
        assert_eq!(json, r#"{"audio":"/tmp/x.wav"}"#);
        let back: NotesGenerateParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn notes_generate_params_full_round_trip() {
        let p = NotesGenerateParams {
            audio: "/tmp/x.wav".into(),
            dialect: Some(NotesDialect::Basic),
            llm_provider: Some("ollama".into()),
            llm_model: Some("gemma3:4b".into()),
            no_diarize: Some(true),
            language: Some("en".into()),
            meeting_id: Some("m1".into()),
        };
        let back: NotesGenerateParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn meeting_round_trips_with_all_fields() {
        let m = Meeting {
            id: "m1".into(),
            title: "Test".into(),
            started_at: "2026-05-05T10:00:00+00:00".into(),
            ended_at: Some("2026-05-05T11:00:00+00:00".into()),
            duration_ms: Some(3_600_000),
            language: "en".into(),
            dir_path: "/tmp/m1".into(),
        };
        let back: Meeting = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn meeting_in_progress_serialises_with_nulls() {
        let m = Meeting {
            id: "m1".into(),
            title: "Test".into(),
            started_at: "2026-05-05T10:00:00+00:00".into(),
            ended_at: None,
            duration_ms: None,
            language: "auto".into(),
            dir_path: "/tmp/m1".into(),
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"ended_at\":null"), "got: {s}");
        assert!(s.contains("\"duration_ms\":null"), "got: {s}");
    }

    #[test]
    fn meeting_config_round_trips_with_optionals_omitted() {
        let cfg = MeetingConfig {
            title: None,
            language: None,
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: MeetingConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn meeting_config_accepts_empty_object() {
        let back: MeetingConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(back.title, None);
        assert_eq!(back.language, None);
    }

    #[test]
    fn notes_generate_params_accepts_meeting_id() {
        let json = r#"{"audio":"/x.wav","meeting_id":"abc"}"#;
        let p: NotesGenerateParams = serde_json::from_str(json).unwrap();
        assert_eq!(p.meeting_id.as_deref(), Some("abc"));
    }

    #[test]
    fn notes_generate_params_accepts_no_meeting_id() {
        let json = r#"{"audio":"/x.wav"}"#;
        let p: NotesGenerateParams = serde_json::from_str(json).unwrap();
        assert_eq!(p.meeting_id, None);
    }

    #[test]
    fn notes_generate_result_emits_meeting_id() {
        let r = NotesGenerateResult {
            primary_file: "/x/notes.md".into(),
            assets: vec!["/x/screenshots/1.jpg".into()],
            meeting_id: "abc".into(),
            transcript_txt_path: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"meeting_id\":\"abc\""), "got: {s}");
    }

    #[test]
    fn notes_generate_result_carries_transcript_txt_path() {
        let r = NotesGenerateResult {
            primary_file: "/x/notes.md".into(),
            assets: vec![],
            meeting_id: "abc".into(),
            transcript_txt_path: Some("/x/transcript.txt".into()),
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(
            s.contains("\"transcript_txt_path\":\"/x/transcript.txt\""),
            "got: {s}"
        );
        let back: NotesGenerateResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn notes_generate_result_omits_transcript_txt_path_when_none() {
        let r = NotesGenerateResult {
            primary_file: "/x/notes.md".into(),
            assets: vec![],
            meeting_id: "abc".into(),
            transcript_txt_path: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(!s.contains("transcript_txt_path"), "got: {s}");
    }

    #[test]
    fn dialect_serializes_as_kebab_case() {
        assert_eq!(
            serde_json::to_string(&NotesDialect::NotionEnhanced).unwrap(),
            r#""notion-enhanced""#
        );
        assert_eq!(
            serde_json::to_string(&NotesDialect::Basic).unwrap(),
            r#""basic""#
        );
    }

    #[test]
    fn job_done_event_round_trips_with_type_tag() {
        let e = Event::JobDone(JobDoneEvent {
            job_id: "abc".into(),
            result: NotesGenerateResult {
                primary_file: "/tmp/notes.md".into(),
                assets: vec!["/tmp/screenshots/a.jpg".into()],
                meeting_id: "m1".into(),
                transcript_txt_path: Some("/tmp/transcript.txt".into()),
            },
        });
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""type":"job.done""#));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn event_unknown_kind_deserializes_as_unknown() {
        // A future engine ships a new event type (`asr.partial`). An old
        // client built against this snapshot of `panops-protocol` must
        // deserialise it as `Event::Unknown(<original value>)` rather
        // than failing — otherwise one new tag tears down every old
        // client's subscription.
        let raw = serde_json::json!({
            "type": "asr.partial",
            "job_id": "abc",
            "text": "hello",
        });
        let parsed: Event =
            serde_json::from_value(raw.clone()).expect("unknown tag deserialises as Unknown");
        match parsed {
            Event::Unknown(v) => assert_eq!(v, raw),
            other => panic!("expected Event::Unknown, got {other:?}"),
        }
    }

    #[test]
    fn event_missing_type_field_is_an_error() {
        // No `type` field at all is still a hard error — the Unknown
        // fallback only catches *unrecognised* tags, not malformed
        // envelopes.
        let raw = serde_json::json!({ "job_id": "abc" });
        let err = serde_json::from_value::<Event>(raw).unwrap_err();
        assert!(
            err.to_string().contains("type"),
            "expected missing-field error, got: {err}"
        );
    }

    #[test]
    fn job_error_event_carries_ipc_error() {
        let e = Event::JobError(JobErrorEvent {
            job_id: "abc".into(),
            error: crate::IpcError::InputNotFound {
                path: "/x.wav".into(),
            },
        });
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""type":"job.error""#));
        assert!(json.contains(r#""kind":"input_not_found""#));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn job_progress_event_round_trips_with_type_tag() {
        let e = Event::JobProgress(JobProgressEvent {
            job_id: "abc".into(),
            stage: "transcribing".into(),
            current: Some(1),
            total: Some(3),
            message: Some("mic track".into()),
        });
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""type":"job.progress""#));
        assert!(json.contains(r#""job_id":"abc""#));
        assert!(json.contains(r#""stage":"transcribing""#));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn meeting_summary_round_trips() {
        let m = MeetingSummary {
            id: "m1".into(),
            title: "Test".into(),
            started_at: "2026-05-02T10:00:00Z".into(),
            ended_at: Some("2026-05-02T10:01:00Z".into()),
            duration_ms: 60_000,
            language: "en".into(),
            has_notes: true,
        };
        let back: MeetingSummary =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    // === Recording IPC type tests (slice 11) ===

    #[test]
    fn recording_start_params_round_trips() {
        let p = RecordingStartParams {
            meeting_id: "m1".into(),
            audio_sources: AudioSourcesWire::SystemAndMic,
            record_video: true,
            screenshot_interval_ms: 500,
            screenshot_threshold: 0.15,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains(r#""record_video":true"#), "got: {json}");
        let back: RecordingStartParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn recording_start_params_accepts_minimal() {
        let json = r#"{"meeting_id":"m1"}"#;
        let p: RecordingStartParams = serde_json::from_str(json).unwrap();
        assert_eq!(p.meeting_id, "m1");
        assert_eq!(p.audio_sources, AudioSourcesWire::SystemAndMic); // default
        assert!(!p.record_video); // default/back-compat
        assert_eq!(p.screenshot_interval_ms, 500); // default
        assert_eq!(p.screenshot_threshold, 0.15); // default
    }

    #[cfg(feature = "domain-conversions")]
    #[test]
    fn recording_start_params_convert_to_domain_capture_config() {
        let p = RecordingStartParams {
            meeting_id: "m1".into(),
            audio_sources: AudioSourcesWire::MicOnly,
            record_video: true,
            screenshot_interval_ms: 250,
            screenshot_threshold: 0.2,
        };
        let cfg = panops_core::capture::CaptureConfig::from(&p);
        assert_eq!(
            cfg.audio_sources,
            panops_core::capture::AudioSources::MicOnly
        );
        assert!(cfg.record_video);
        assert_eq!(cfg.screenshot_interval_ms, 250);
        assert_eq!(cfg.screenshot_threshold, 0.2);

        let wire = AudioSourcesWire::from(panops_core::capture::AudioSources::SystemOnly);
        assert_eq!(wire, AudioSourcesWire::SystemOnly);
    }

    #[test]
    fn meeting_delete_video_shapes_round_trip() {
        let params = MeetingDeleteVideoParams {
            meeting_id: "m1".into(),
        };
        assert_eq!(
            serde_json::to_string(&params).unwrap(),
            r#"{"meeting_id":"m1"}"#
        );

        let result = MeetingDeleteVideoResult {
            deleted: true,
            freed_bytes: 42,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(json, r#"{"deleted":true,"freed_bytes":42}"#);
        let back: MeetingDeleteVideoResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, result);
    }

    #[test]
    fn recording_accepted_round_trips() {
        let r = RecordingAccepted {
            recording_id: "rec123".into(),
        };
        let back =
            serde_json::from_str::<RecordingAccepted>(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn recording_stop_params_round_trips() {
        let p = RecordingStopParams {
            recording_id: "rec123".into(),
        };
        let back = serde_json::from_str::<RecordingStopParams>(&serde_json::to_string(&p).unwrap())
            .unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn recording_stopped_round_trips() {
        let r = RecordingStopped {
            system_audio_path: Some("/tmp/system.wav".into()),
            mic_audio_path: Some("/tmp/mic.wav".into()),
            screenshot_paths: vec!["/tmp/screenshots/001.jpg".into()],
            duration_ms: 60_000,
        };
        let back =
            serde_json::from_str::<RecordingStopped>(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn screenshot_event_round_trips_with_type_tag() {
        let e = Event::Screenshot(ScreenshotEvent {
            meeting_id: "m1".into(),
            timestamp_ms: 12345,
            path: "/tmp/screenshots/001.jpg".into(),
        });
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""type":"screenshot""#));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn recording_progress_event_round_trips_with_type_tag() {
        let e = Event::RecordingProgress(RecordingProgressEvent {
            meeting_id: "m1".into(),
            bytes_captured: 1024,
            duration_ms: 5000,
        });
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""type":"recording.progress""#));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn audio_sources_wire_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&AudioSourcesWire::SystemOnly).unwrap(),
            r#""system_only""#
        );
        assert_eq!(
            serde_json::to_string(&AudioSourcesWire::MicOnly).unwrap(),
            r#""mic_only""#
        );
        assert_eq!(
            serde_json::to_string(&AudioSourcesWire::SystemAndMic).unwrap(),
            r#""system_and_mic""#
        );
    }
}
