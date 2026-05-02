//! JSON-RPC method params/results and WebSocket event payloads.
//!
//! Method names appear with an `ipc.` namespace at the wire level
//! (jsonrpsee `#[rpc(namespace = "ipc")]`). Param/result types are pure
//! data — no method routing happens in this crate.

use serde::{Deserialize, Deserializer, Serialize};

/// Type-tagged so the same `events` subscription multiplexes job lifecycle.
/// Future event kinds (`asr.partial`, `screenshot`, ...) extend this enum;
/// clients running an older `panops-protocol` deserialise the new tag as
/// `Event::Unknown(<original JSON>)` and keep the subscription alive.
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
pub struct JobAccepted {
    pub job_id: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetingSummary {
    pub id: String,
    pub title: String,
    /// RFC3339 timestamp. Kept as `String` (not `chrono::DateTime`) so this
    /// crate stays free of date-time deps; non-Rust consumers don't need
    /// a Rust-specific time crate to consume it.
    pub started_at: String,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_generate_params_minimal_round_trip() {
        let p = NotesGenerateParams {
            audio: "/tmp/x.wav".into(),
            dialect: None,
            llm_provider: None,
            llm_model: None,
            no_diarize: None,
            language: None,
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
        };
        let back: NotesGenerateParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
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
    fn meeting_summary_round_trips() {
        let m = MeetingSummary {
            id: "m1".into(),
            title: "Test".into(),
            started_at: "2026-05-02T10:00:00Z".into(),
            duration_ms: 60_000,
        };
        let back: MeetingSummary =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}
