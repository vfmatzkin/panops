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
    /// Optional organization space assignment. `None` means the meeting
    /// is in the implicit Inbox/unsorted bucket.
    #[serde(default)]
    pub space_id: Option<String>,
    /// Optional organization project assignment.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Tag ids assigned to this meeting.
    #[serde(default)]
    pub tags: Vec<String>,
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
            space_id: value.space_id,
            project_id: value.project_id,
            tags: value.tags,
        }
    }
}

/// Wire organization space.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub position: i64,
}

/// Wire organization project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub space_id: String,
    pub name: String,
    pub position: i64,
}

/// Wire organization tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tag {
    pub id: String,
    pub name: String,
}

#[cfg(feature = "domain-conversions")]
impl From<panops_core::storage::Space> for Space {
    fn from(value: panops_core::storage::Space) -> Self {
        Self {
            id: value.id,
            name: value.name,
            position: value.position,
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<Space> for panops_core::storage::Space {
    fn from(value: Space) -> Self {
        Self {
            id: value.id,
            name: value.name,
            position: value.position,
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<panops_core::storage::Project> for Project {
    fn from(value: panops_core::storage::Project) -> Self {
        Self {
            id: value.id,
            space_id: value.space_id,
            name: value.name,
            position: value.position,
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<Project> for panops_core::storage::Project {
    fn from(value: Project) -> Self {
        Self {
            id: value.id,
            space_id: value.space_id,
            name: value.name,
            position: value.position,
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<panops_core::storage::Tag> for Tag {
    fn from(value: panops_core::storage::Tag) -> Self {
        Self {
            id: value.id,
            name: value.name,
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<Tag> for panops_core::storage::Tag {
    fn from(value: Tag) -> Self {
        Self {
            id: value.id,
            name: value.name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceCreateParams {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceListResult {
    pub spaces: Vec<Space>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceRenameParams {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceDeleteParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectCreateParams {
    pub space_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectListResult {
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRenameParams {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectDeleteParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagCreateParams {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagListResult {
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagDeleteParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagAssignParams {
    pub meeting_id: String,
    pub tag_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeetingAssignParams {
    pub meeting_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeetingListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_id: Option<String>,
    #[serde(default)]
    pub unsorted: bool,
}

#[cfg(feature = "domain-conversions")]
impl From<MeetingListParams> for panops_core::storage::MeetingListFilter {
    fn from(value: MeetingListParams) -> Self {
        Self {
            space_id: value.space_id,
            project_id: value.project_id,
            tag_id: value.tag_id,
            unsorted: value.unsorted,
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<panops_core::storage::MeetingListFilter> for MeetingListParams {
    fn from(value: panops_core::storage::MeetingListFilter) -> Self {
        Self {
            space_id: value.space_id,
            project_id: value.project_id,
            tag_id: value.tag_id,
            unsorted: value.unsorted,
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

/// Screen target selection for `recording.start`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureTarget {
    Display {
        #[serde(default)]
        display_id: u32,
    },
    Window {
        window_id: u32,
    },
    App {
        bundle_id: String,
    },
    Region {
        #[serde(default)]
        display_id: u32,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    },
}

impl Default for CaptureTarget {
    fn default() -> Self {
        CaptureTarget::Display { display_id: 0 }
    }
}

/// Capturable window metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowInfo {
    pub window_id: u32,
    pub app_name: String,
    pub title: String,
}

/// Params for `ipc.capture.windows`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureWindowsParams {}

/// Result for `ipc.capture.windows`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureWindowsResult {
    pub windows: Vec<WindowInfo>,
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
    /// Whether to enqueue the notes pipeline automatically after a successful
    /// `recording.stop`. The serde default is `false` only for backwards
    /// compatibility with older clients that omit the field; the app's UX
    /// default is enabled and sends `true` explicitly.
    #[serde(default)]
    pub auto_generate_notes: bool,
    #[serde(default = "default_screenshot_interval")]
    pub screenshot_interval_ms: u64,
    #[serde(default = "default_screenshot_threshold")]
    pub screenshot_threshold: f32,
    /// Screen target to capture. Defaults to full-display capture.
    #[serde(default)]
    pub capture_target: CaptureTarget,
    /// Output width in px. `None` = native. Set both width+height or neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Output height in px. `None` = native.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
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
impl From<CaptureTarget> for panops_core::capture::CaptureTarget {
    fn from(value: CaptureTarget) -> Self {
        match value {
            CaptureTarget::Display { display_id } => Self::Display { display_id },
            CaptureTarget::Window { window_id } => Self::Window { window_id },
            CaptureTarget::App { bundle_id } => Self::App { bundle_id },
            CaptureTarget::Region {
                display_id,
                x,
                y,
                w,
                h,
            } => Self::Region {
                display_id,
                x,
                y,
                w,
                h,
            },
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<panops_core::capture::CaptureTarget> for CaptureTarget {
    fn from(value: panops_core::capture::CaptureTarget) -> Self {
        match value {
            panops_core::capture::CaptureTarget::Display { display_id } => {
                Self::Display { display_id }
            }
            panops_core::capture::CaptureTarget::Window { window_id } => Self::Window { window_id },
            panops_core::capture::CaptureTarget::App { bundle_id } => Self::App { bundle_id },
            panops_core::capture::CaptureTarget::Region {
                display_id,
                x,
                y,
                w,
                h,
            } => Self::Region {
                display_id,
                x,
                y,
                w,
                h,
            },
        }
    }
}

#[cfg(feature = "domain-conversions")]
impl From<panops_core::capture::WindowInfo> for WindowInfo {
    fn from(value: panops_core::capture::WindowInfo) -> Self {
        Self {
            window_id: value.window_id,
            app_name: value.app_name,
            title: value.title,
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
            capture_target: value.capture_target.clone().into(),
            width: value.width,
            height: value.height,
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
    /// Engine-issued notes job id when `auto_generate_notes` was set on the
    /// recording AND a notes provider was ready at stop. `None` when auto-notes
    /// wasn't requested, or was requested but compute wasn't ready (warmup / no
    /// provider) — clients detect "auto requested but no job" and surface a
    /// deferred hint. `#[serde(default)]` + skip-when-none keeps the field
    /// forward-compatible: older payloads (no field) still decode all-default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_job_id: Option<String>,
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
            space_id: Some("space_1".into()),
            project_id: Some("project_1".into()),
            tags: vec!["tag_1".into()],
        };
        let back: MeetingSummary =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn meeting_summary_accepts_legacy_payload_without_org_fields() {
        let json = r#"{"id":"m1","title":"Test","started_at":"2026-05-02T10:00:00Z","ended_at":null,"duration_ms":0,"language":"auto","has_notes":false}"#;
        let m: MeetingSummary = serde_json::from_str(json).unwrap();
        assert_eq!(m.space_id, None);
        assert_eq!(m.project_id, None);
        assert!(m.tags.is_empty());
    }

    #[test]
    fn organization_wire_shapes_round_trip() {
        let space = Space {
            id: "space_1".into(),
            name: "Work".into(),
            position: 0,
        };
        assert_eq!(
            serde_json::to_string(&space).unwrap(),
            r#"{"id":"space_1","name":"Work","position":0}"#
        );
        let spaces = SpaceListResult {
            spaces: vec![space.clone()],
        };
        assert_eq!(
            serde_json::to_string(&spaces).unwrap(),
            r#"{"spaces":[{"id":"space_1","name":"Work","position":0}]}"#
        );
        assert_eq!(
            serde_json::from_str::<SpaceListResult>(&serde_json::to_string(&spaces).unwrap())
                .unwrap(),
            spaces
        );

        let project = Project {
            id: "project_1".into(),
            space_id: "space_1".into(),
            name: "Panops".into(),
            position: 1,
        };
        let projects = ProjectListResult {
            projects: vec![project],
        };
        assert_eq!(
            serde_json::to_string(&projects).unwrap(),
            r#"{"projects":[{"id":"project_1","space_id":"space_1","name":"Panops","position":1}]}"#
        );
        assert_eq!(
            serde_json::from_str::<ProjectListResult>(&serde_json::to_string(&projects).unwrap())
                .unwrap(),
            projects
        );

        let tag = Tag {
            id: "tag_1".into(),
            name: "follow-up".into(),
        };
        let tags = TagListResult { tags: vec![tag] };
        assert_eq!(
            serde_json::to_string(&tags).unwrap(),
            r#"{"tags":[{"id":"tag_1","name":"follow-up"}]}"#
        );
        assert_eq!(
            serde_json::from_str::<TagListResult>(&serde_json::to_string(&tags).unwrap()).unwrap(),
            tags
        );
    }

    #[test]
    fn organization_param_shapes_round_trip() {
        assert_eq!(
            serde_json::to_string(&SpaceCreateParams {
                name: "Work".into()
            })
            .unwrap(),
            r#"{"name":"Work"}"#
        );
        assert_eq!(
            serde_json::to_string(&SpaceRenameParams {
                id: "space_1".into(),
                name: "Study".into()
            })
            .unwrap(),
            r#"{"id":"space_1","name":"Study"}"#
        );
        assert_eq!(
            serde_json::to_string(&SpaceDeleteParams {
                id: "space_1".into()
            })
            .unwrap(),
            r#"{"id":"space_1"}"#
        );
        assert_eq!(
            serde_json::to_string(&ProjectCreateParams {
                space_id: "space_1".into(),
                name: "Panops".into()
            })
            .unwrap(),
            r#"{"space_id":"space_1","name":"Panops"}"#
        );
        assert_eq!(
            serde_json::to_string(&ProjectListParams {
                space_id: Some("space_1".into())
            })
            .unwrap(),
            r#"{"space_id":"space_1"}"#
        );
        assert_eq!(
            serde_json::to_string(&ProjectRenameParams {
                id: "project_1".into(),
                name: "Phase B".into()
            })
            .unwrap(),
            r#"{"id":"project_1","name":"Phase B"}"#
        );
        assert_eq!(
            serde_json::to_string(&ProjectDeleteParams {
                id: "project_1".into()
            })
            .unwrap(),
            r#"{"id":"project_1"}"#
        );
        assert_eq!(
            serde_json::to_string(&TagCreateParams {
                name: "follow-up".into()
            })
            .unwrap(),
            r#"{"name":"follow-up"}"#
        );
        assert_eq!(
            serde_json::to_string(&TagDeleteParams { id: "tag_1".into() }).unwrap(),
            r#"{"id":"tag_1"}"#
        );
        assert_eq!(
            serde_json::to_string(&TagAssignParams {
                meeting_id: "m1".into(),
                tag_id: "tag_1".into()
            })
            .unwrap(),
            r#"{"meeting_id":"m1","tag_id":"tag_1"}"#
        );
        assert_eq!(
            serde_json::to_string(&MeetingAssignParams {
                meeting_id: "m1".into(),
                space_id: Some("space_1".into()),
                project_id: None,
            })
            .unwrap(),
            r#"{"meeting_id":"m1","space_id":"space_1"}"#
        );
        assert_eq!(
            serde_json::to_string(&MeetingListParams {
                space_id: Some("space_1".into()),
                project_id: None,
                tag_id: Some("tag_1".into()),
                unsorted: false,
            })
            .unwrap(),
            r#"{"space_id":"space_1","tag_id":"tag_1","unsorted":false}"#
        );
        let empty_list_params: MeetingListParams = serde_json::from_str("{}").unwrap();
        assert_eq!(empty_list_params, MeetingListParams::default());
    }

    #[cfg(feature = "domain-conversions")]
    #[test]
    fn organization_wire_converts_both_directions() {
        let domain_space = panops_core::storage::Space {
            id: "space_1".into(),
            name: "Work".into(),
            position: 0,
        };
        let wire_space = Space::from(domain_space.clone());
        assert_eq!(wire_space.id, "space_1");
        assert_eq!(panops_core::storage::Space::from(wire_space), domain_space);

        let domain_project = panops_core::storage::Project {
            id: "project_1".into(),
            space_id: "space_1".into(),
            name: "Panops".into(),
            position: 1,
        };
        let wire_project = Project::from(domain_project.clone());
        assert_eq!(wire_project.space_id, "space_1");
        assert_eq!(
            panops_core::storage::Project::from(wire_project),
            domain_project
        );

        let domain_tag = panops_core::storage::Tag {
            id: "tag_1".into(),
            name: "follow-up".into(),
        };
        let wire_tag = Tag::from(domain_tag.clone());
        assert_eq!(wire_tag.name, "follow-up");
        assert_eq!(panops_core::storage::Tag::from(wire_tag), domain_tag);
    }

    #[cfg(feature = "domain-conversions")]
    #[test]
    fn meeting_list_params_convert_to_filter_and_back() {
        let params = MeetingListParams {
            space_id: Some("space_1".into()),
            project_id: Some("project_1".into()),
            tag_id: Some("tag_1".into()),
            unsorted: true,
        };
        let filter = panops_core::storage::MeetingListFilter::from(params.clone());
        assert_eq!(filter.space_id.as_deref(), Some("space_1"));
        assert_eq!(filter.project_id.as_deref(), Some("project_1"));
        assert_eq!(filter.tag_id.as_deref(), Some("tag_1"));
        assert!(filter.unsorted);
        assert_eq!(MeetingListParams::from(filter), params);
    }

    // === Recording IPC type tests (slice 11) ===

    #[test]
    fn recording_start_params_round_trips() {
        let p = RecordingStartParams {
            meeting_id: "m1".into(),
            audio_sources: AudioSourcesWire::SystemAndMic,
            record_video: true,
            auto_generate_notes: true,
            screenshot_interval_ms: 500,
            screenshot_threshold: 0.15,
            capture_target: CaptureTarget::Window { window_id: 42 },
            width: None,
            height: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains(r#""record_video":true"#), "got: {json}");
        assert!(
            json.contains(r#""auto_generate_notes":true"#),
            "got: {json}"
        );
        assert!(
            json.contains(r#""capture_target":{"kind":"window","window_id":42}"#),
            "got: {json}"
        );
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
        assert!(!p.auto_generate_notes); // default/back-compat
        assert_eq!(p.screenshot_interval_ms, 500); // default
        assert_eq!(p.screenshot_threshold, 0.15); // default
        assert_eq!(p.capture_target, CaptureTarget::Display { display_id: 0 }); // default
    }

    #[cfg(feature = "domain-conversions")]
    #[test]
    fn recording_start_params_convert_to_domain_capture_config() {
        let p = RecordingStartParams {
            meeting_id: "m1".into(),
            audio_sources: AudioSourcesWire::MicOnly,
            record_video: true,
            auto_generate_notes: true,
            screenshot_interval_ms: 250,
            screenshot_threshold: 0.2,
            capture_target: CaptureTarget::Window { window_id: 99 },
            width: None,
            height: None,
        };
        let cfg = panops_core::capture::CaptureConfig::from(&p);
        assert_eq!(
            cfg.audio_sources,
            panops_core::capture::AudioSources::MicOnly
        );
        assert!(cfg.record_video);
        assert_eq!(cfg.screenshot_interval_ms, 250);
        assert_eq!(cfg.screenshot_threshold, 0.2);
        assert_eq!(
            cfg.capture_target,
            panops_core::capture::CaptureTarget::Window { window_id: 99 }
        );

        let wire = AudioSourcesWire::from(panops_core::capture::AudioSources::SystemOnly);
        assert_eq!(wire, AudioSourcesWire::SystemOnly);
        let wire_target =
            CaptureTarget::from(panops_core::capture::CaptureTarget::Window { window_id: 7 });
        assert_eq!(wire_target, CaptureTarget::Window { window_id: 7 });
    }

    #[test]
    fn capture_target_wire_contracts_are_snake_case_tagged() {
        assert_eq!(
            serde_json::to_string(&CaptureTarget::Display { display_id: 0 }).unwrap(),
            r#"{"kind":"display","display_id":0}"#
        );
        assert_eq!(
            serde_json::to_string(&CaptureTarget::Window { window_id: 42 }).unwrap(),
            r#"{"kind":"window","window_id":42}"#
        );
        let defaulted: RecordingStartParams =
            serde_json::from_str(r#"{"meeting_id":"m1"}"#).unwrap();
        assert_eq!(
            defaulted.capture_target,
            CaptureTarget::Display { display_id: 0 }
        );
    }

    #[test]
    fn capture_target_wire_new_variants_round_trip() {
        for t in [
            CaptureTarget::Display { display_id: 0 },
            CaptureTarget::Window { window_id: 42 },
            CaptureTarget::App {
                bundle_id: "com.apple.Safari".into(),
            },
            CaptureTarget::Region {
                display_id: 1,
                x: 10,
                y: 20,
                w: 640,
                h: 480,
            },
        ] {
            let back: CaptureTarget =
                serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
            assert_eq!(back, t);
        }
    }

    #[test]
    fn capture_target_wire_display_back_compat() {
        // Old clients send {"kind":"display"} with no display_id.
        let t: CaptureTarget = serde_json::from_str(r#"{"kind":"display"}"#).unwrap();
        assert_eq!(t, CaptureTarget::Display { display_id: 0 });
    }

    #[test]
    fn recording_start_params_carry_resolution() {
        let json = r#"{"meeting_id":"m1","width":1280,"height":720}"#;
        let p: RecordingStartParams = serde_json::from_str(json).unwrap();
        assert_eq!(p.width, Some(1280));
        assert_eq!(p.height, Some(720));
    }

    #[test]
    fn recording_start_params_resolution_defaults_none() {
        let p: RecordingStartParams = serde_json::from_str(r#"{"meeting_id":"m1"}"#).unwrap();
        assert_eq!(p.width, None);
        assert_eq!(p.height, None);
    }

    #[test]
    fn capture_windows_shapes_round_trip() {
        let params = CaptureWindowsParams {};
        assert_eq!(serde_json::to_string(&params).unwrap(), r#"{}"#);

        let result = CaptureWindowsResult {
            windows: vec![WindowInfo {
                window_id: 42,
                app_name: "Safari".into(),
                title: "Panops".into(),
            }],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(
            json,
            r#"{"windows":[{"window_id":42,"app_name":"Safari","title":"Panops"}]}"#
        );
        let back: CaptureWindowsResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, result);
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
            notes_job_id: Some("job-123".into()),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""notes_job_id":"job-123""#), "got: {json}");
        let back = serde_json::from_str::<RecordingStopped>(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn recording_stopped_decodes_without_notes_job_id() {
        // Older engine payloads (pre auto-notes-observability) omit the field;
        // they must still decode with `notes_job_id == None`.
        let json = r#"{"system_audio_path":"/tmp/system.wav","mic_audio_path":null,"screenshot_paths":[],"duration_ms":1000}"#;
        let r: RecordingStopped = serde_json::from_str(json).unwrap();
        assert_eq!(r.notes_job_id, None);
        // And when absent, it must not be emitted on the wire.
        let back = serde_json::to_string(&r).unwrap();
        assert!(!back.contains("notes_job_id"), "got: {back}");
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
