//! Wire error type. Round-trips through serde; forward-compatible via
//! `#[serde(other)]` `Unknown` variant.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, Error, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IpcError {
    #[error("input not found: {path}")]
    InputNotFound { path: String },
    #[error("invalid input: {message}")]
    InvalidInput { message: String },
    #[error("provider unavailable: {message}")]
    ProviderUnavailable { message: String },
    #[error("internal: {message}")]
    Internal { message: String },
    #[error("cancelled")]
    Cancelled,
    /// Unknown kind — used as the deserialization fallback so old clients
    /// never hard-fail on a future engine that adds new variants.
    #[serde(other)]
    #[error("unknown error kind (forward-compat fallback)")]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_not_found_round_trips() {
        let e = IpcError::InputNotFound {
            path: "/tmp/missing.wav".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""kind":"input_not_found""#));
        assert!(json.contains(r#""path":"/tmp/missing.wav""#));
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn invalid_input_round_trips() {
        let e = IpcError::InvalidInput {
            message: "bad".into(),
        };
        let back: IpcError = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn provider_unavailable_round_trips() {
        let e = IpcError::ProviderUnavailable {
            message: "down".into(),
        };
        let back: IpcError = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn internal_round_trips() {
        let e = IpcError::Internal {
            message: "oops".into(),
        };
        let back: IpcError = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn cancelled_serializes_as_unit() {
        let e = IpcError::Cancelled;
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, r#"{"kind":"cancelled"}"#);
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn unknown_kind_deserializes_as_unknown_not_error() {
        // Forward-compat: a future engine adds a new variant; old clients
        // must NOT fail the whole RPC response.
        let json = r#"{"kind":"future_variant","extra":"ignored"}"#;
        let back: IpcError = serde_json::from_str(json).unwrap();
        assert_eq!(back, IpcError::Unknown);
    }

    #[test]
    fn display_includes_kind_message() {
        let e = IpcError::InputNotFound {
            path: "/x.wav".into(),
        };
        assert!(format!("{e}").contains("input not found"));
        assert!(format!("{e}").contains("/x.wav"));
    }
}

#[cfg(feature = "domain-conversions")]
mod from_domain {
    use super::IpcError;
    use panops_core::asr::AsrError;
    use panops_core::diar::DiarError;
    use panops_core::exporter::ExportError;
    use panops_core::llm::LlmError;
    use panops_core::notes::error::NotesError;
    use panops_core::storage::StorageError;
    use panops_core::vad::VadError;

    impl From<AsrError> for IpcError {
        fn from(e: AsrError) -> Self {
            match e {
                AsrError::AudioNotFound(p) => IpcError::InputNotFound {
                    path: p.display().to_string(),
                },
                AsrError::InvalidAudio(m) => IpcError::InvalidInput { message: m },
                AsrError::Model(m) | AsrError::Transcription(m) => {
                    IpcError::Internal { message: m }
                }
                AsrError::Io(io) => IpcError::Internal {
                    message: io.to_string(),
                },
            }
        }
    }

    impl From<DiarError> for IpcError {
        fn from(e: DiarError) -> Self {
            match e {
                DiarError::AudioNotFound(p) => IpcError::InputNotFound {
                    path: p.display().to_string(),
                },
                DiarError::InvalidAudio(m) => IpcError::InvalidInput { message: m },
                DiarError::Model(m) | DiarError::Diarization(m) => {
                    IpcError::Internal { message: m }
                }
                DiarError::Io(io) => IpcError::Internal {
                    message: io.to_string(),
                },
            }
        }
    }

    impl From<LlmError> for IpcError {
        fn from(e: LlmError) -> Self {
            match e {
                LlmError::Network(m) | LlmError::Provider(m) => {
                    IpcError::ProviderUnavailable { message: m }
                }
                LlmError::InvalidSchema { expected, got } => IpcError::Internal {
                    message: format!("schema mismatch: expected {expected}, got {got}"),
                },
                LlmError::EmptyResponse => IpcError::ProviderUnavailable {
                    message: "empty LLM response".into(),
                },
                LlmError::Cancelled => IpcError::Cancelled,
            }
        }
    }

    impl From<NotesError> for IpcError {
        fn from(e: NotesError) -> Self {
            match e {
                NotesError::EmptyTranscript => IpcError::InvalidInput {
                    message: "empty transcript".into(),
                },
                NotesError::Llm(le) => le.into(),
                NotesError::SchemaMismatch { stage, detail } => IpcError::Internal {
                    message: format!("schema mismatch in stage {stage}: {detail}"),
                },
                NotesError::InvalidInput(m) => IpcError::InvalidInput { message: m },
            }
        }
    }

    impl From<ExportError> for IpcError {
        fn from(e: ExportError) -> Self {
            match e {
                // The destination directory came from caller input
                // (`notes.generate`'s computed `out_dir`). A failure here
                // means the path itself was rejected — surface as
                // invalid-input with an opaque message so the wire never
                // echoes the FS layout the caller probed.
                ExportError::InvalidDest(_) => IpcError::InvalidInput {
                    message: "invalid export destination".into(),
                },
                // Io / Render are server-side conditions (disk, template
                // engine). Keep them opaque on the wire; full detail is
                // emitted via `tracing::error!` at the call site.
                ExportError::Io(_) | ExportError::Render(_) => IpcError::Internal {
                    message: "export failed".into(),
                },
            }
        }
    }

    impl From<StorageError> for IpcError {
        fn from(e: StorageError) -> Self {
            match e {
                // `path` carries `<kind>/<id>` so clients can tell which
                // entity was missing. `kind` is a `&'static` from a
                // closed set ("meeting" | "note"); the id was the input
                // they just passed, so this is not a new info leak.
                StorageError::NotFound { id, kind } => IpcError::InputNotFound {
                    path: format!("{kind}/{id}"),
                },
                StorageError::AlreadyExists { kind, .. } => IpcError::InvalidInput {
                    message: format!("{kind} already exists"),
                },
                StorageError::UniqueConflict { kind, field, .. } => IpcError::InvalidInput {
                    message: format!("{kind}.{field} already in use"),
                },
                // Internal-state conditions: the wire stays opaque so
                // version numbers / SQL fragments / FS paths never reach
                // the client. Full detail is logged at the call site
                // (handlers.rs) where `tracing` is available — this
                // crate is `tracing`-free to keep transport types thin.
                StorageError::SchemaMismatch { .. } => IpcError::Internal {
                    message: "storage schema mismatch".into(),
                },
                StorageError::Io { .. } => IpcError::Internal {
                    message: "storage io error".into(),
                },
                StorageError::Sql { .. } => IpcError::Internal {
                    message: "storage error".into(),
                },
            }
        }
    }

    impl From<VadError> for IpcError {
        fn from(e: VadError) -> Self {
            match e {
                VadError::InvalidAudio(m) => IpcError::InvalidInput { message: m },
                VadError::Model(_) => IpcError::Internal {
                    message: "vad model error".into(),
                },
                VadError::Io { .. } => IpcError::Internal {
                    message: "vad io error".into(),
                },
            }
        }
    }
}

#[cfg(all(test, feature = "domain-conversions"))]
mod from_domain_tests {
    use super::IpcError;
    use panops_core::asr::AsrError;
    use panops_core::diar::DiarError;
    use panops_core::exporter::ExportError;
    use panops_core::llm::LlmError;
    use panops_core::notes::error::NotesError;
    use panops_core::vad::VadError;
    use std::path::PathBuf;

    #[test]
    fn asr_audio_not_found_maps_to_input_not_found() {
        let e: IpcError = AsrError::AudioNotFound(PathBuf::from("/x.wav")).into();
        assert!(matches!(e, IpcError::InputNotFound { .. }));
        if let IpcError::InputNotFound { path } = e {
            assert!(path.contains("/x.wav"));
        }
    }

    #[test]
    fn asr_invalid_audio_maps_to_invalid_input() {
        let e: IpcError = AsrError::InvalidAudio("bad header".into()).into();
        assert!(matches!(e, IpcError::InvalidInput { ref message } if message == "bad header"));
    }

    #[test]
    fn asr_model_and_transcription_map_to_internal() {
        let e1: IpcError = AsrError::Model("m".into()).into();
        let e2: IpcError = AsrError::Transcription("t".into()).into();
        assert!(matches!(e1, IpcError::Internal { .. }));
        assert!(matches!(e2, IpcError::Internal { .. }));
    }

    #[test]
    fn asr_io_maps_to_internal() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope");
        let e: IpcError = AsrError::Io(io).into();
        assert!(matches!(e, IpcError::Internal { .. }));
    }

    #[test]
    fn diar_audio_not_found_maps_to_input_not_found() {
        let e: IpcError = DiarError::AudioNotFound(PathBuf::from("/x.wav")).into();
        assert!(matches!(e, IpcError::InputNotFound { .. }));
    }

    #[test]
    fn diar_invalid_audio_maps_to_invalid_input() {
        let e: IpcError = DiarError::InvalidAudio("bad".into()).into();
        assert!(matches!(e, IpcError::InvalidInput { .. }));
    }

    #[test]
    fn diar_model_and_diarization_map_to_internal() {
        let e1: IpcError = DiarError::Model("m".into()).into();
        let e2: IpcError = DiarError::Diarization("d".into()).into();
        assert!(matches!(e1, IpcError::Internal { .. }));
        assert!(matches!(e2, IpcError::Internal { .. }));
    }

    #[test]
    fn llm_network_and_provider_map_to_provider_unavailable() {
        let e1: IpcError = LlmError::Network("timeout".into()).into();
        let e2: IpcError = LlmError::Provider("down".into()).into();
        assert!(matches!(e1, IpcError::ProviderUnavailable { .. }));
        assert!(matches!(e2, IpcError::ProviderUnavailable { .. }));
    }

    #[test]
    fn llm_invalid_schema_maps_to_internal_with_context() {
        let e: IpcError = LlmError::InvalidSchema {
            expected: "object".into(),
            got: "string".into(),
        }
        .into();
        assert!(matches!(e, IpcError::Internal { ref message }
                if message.contains("expected object") && message.contains("got string")));
    }

    #[test]
    fn llm_empty_response_maps_to_provider_unavailable() {
        let e: IpcError = LlmError::EmptyResponse.into();
        assert!(matches!(e, IpcError::ProviderUnavailable { .. }));
    }

    #[test]
    fn llm_cancelled_maps_to_cancelled() {
        let e: IpcError = LlmError::Cancelled.into();
        assert_eq!(e, IpcError::Cancelled);
    }

    #[test]
    fn notes_empty_transcript_maps_to_invalid_input() {
        let e: IpcError = NotesError::EmptyTranscript.into();
        assert!(matches!(e, IpcError::InvalidInput { .. }));
    }

    #[test]
    fn notes_llm_recurses_into_llm_mapping() {
        let e: IpcError = NotesError::Llm(LlmError::Cancelled).into();
        assert_eq!(e, IpcError::Cancelled);
    }

    #[test]
    fn notes_schema_mismatch_maps_to_internal_with_stage() {
        let e: IpcError = NotesError::SchemaMismatch {
            stage: "section",
            detail: "missing key".into(),
        }
        .into();
        assert!(matches!(e, IpcError::Internal { ref message }
                if message.contains("section") && message.contains("missing key")));
    }

    #[test]
    fn notes_invalid_input_maps_to_invalid_input() {
        let e: IpcError = NotesError::InvalidInput("bad".into()).into();
        assert!(matches!(e, IpcError::InvalidInput { .. }));
    }

    #[test]
    fn export_invalid_dest_maps_to_invalid_input() {
        let e: IpcError = ExportError::InvalidDest("not a directory".into()).into();
        assert!(matches!(e, IpcError::InvalidInput { .. }));
    }

    #[test]
    fn export_io_maps_to_internal() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let e: IpcError = ExportError::Io(io).into();
        assert!(matches!(e, IpcError::Internal { .. }));
    }

    #[test]
    fn export_render_maps_to_internal() {
        let e: IpcError = ExportError::Render("template failure".into()).into();
        assert!(matches!(e, IpcError::Internal { .. }));
    }

    #[test]
    fn export_invalid_dest_does_not_leak_caller_path() {
        // Wire boundary: an attacker probing `notes.generate` with a
        // crafted destination must not see the path echoed back.
        let e: IpcError = ExportError::InvalidDest("/etc/passwd".into()).into();
        if let IpcError::InvalidInput { message } = e {
            assert!(
                !message.contains("/etc/passwd"),
                "leaked caller path: {message}"
            );
        } else {
            panic!("expected InvalidInput");
        }
    }

    use panops_core::storage::StorageError;

    #[test]
    fn storage_not_found_maps_to_input_not_found_with_kind_and_id() {
        let e: IpcError = StorageError::NotFound {
            id: "abc".into(),
            kind: "meeting",
        }
        .into();
        let IpcError::InputNotFound { path } = e else {
            panic!("expected InputNotFound");
        };
        assert!(path.contains("meeting"), "got: {path}");
        assert!(path.contains("abc"), "got: {path}");
    }

    #[test]
    fn storage_unique_conflict_maps_to_invalid_input_without_value_leak() {
        let e: IpcError = StorageError::UniqueConflict {
            kind: "meeting",
            field: "dir_path",
            value: "/Users/fran/Library/Application Support/panops/meetings/abc".into(),
        }
        .into();
        let IpcError::InvalidInput { message } = e else {
            panic!("expected InvalidInput");
        };
        assert!(message.contains("meeting"), "got: {message}");
        assert!(message.contains("dir_path"), "got: {message}");
        // The path itself must not leak — only kind+field on the wire.
        assert!(!message.contains("/Users/fran"), "value leaked: {message}");
    }

    #[test]
    fn storage_already_exists_maps_to_invalid_input_without_id_leak() {
        let e: IpcError = StorageError::AlreadyExists {
            id: "secret-id-do-not-leak".into(),
            kind: "note",
        }
        .into();
        let IpcError::InvalidInput { message } = e else {
            panic!("expected InvalidInput");
        };
        assert!(message.contains("note"), "got: {message}");
        assert!(
            !message.contains("secret-id-do-not-leak"),
            "id leaked: {message}"
        );
    }

    #[test]
    fn storage_schema_mismatch_maps_to_internal_without_version_leak() {
        let e: IpcError = StorageError::SchemaMismatch {
            actual: 2,
            expected: 1,
        }
        .into();
        let IpcError::Internal { message } = e else {
            panic!("expected Internal");
        };
        // No version-number leak in wire message.
        assert_eq!(message, "storage schema mismatch");
    }

    #[test]
    fn storage_io_maps_to_internal_without_path_leak() {
        let io = std::io::Error::other("/some/secret/path failed");
        let e: IpcError = StorageError::Io { source: io }.into();
        let IpcError::Internal { message } = e else {
            panic!("expected Internal");
        };
        assert_eq!(message, "storage io error");
        assert!(!message.contains("/some/secret/path"));
    }

    #[test]
    fn storage_sql_maps_to_internal_without_query_leak() {
        let e: IpcError = StorageError::Sql {
            message: "near 'SELECT': syntax error".into(),
        }
        .into();
        let IpcError::Internal { message } = e else {
            panic!("expected Internal");
        };
        assert_eq!(message, "storage error");
        assert!(!message.contains("SELECT"));
    }

    #[test]
    fn vad_invalid_audio_maps_to_invalid_input_with_message() {
        let e: IpcError = VadError::InvalidAudio("expected 16 kHz, got 8 kHz".into()).into();
        let IpcError::InvalidInput { message } = e else {
            panic!("expected InvalidInput");
        };
        assert!(message.contains("16 kHz"), "got: {message}");
    }

    #[test]
    fn vad_model_does_not_leak_path_or_mutex_detail() {
        let e: IpcError = VadError::Model(
            "/Users/fran/Library/.../models/ggml-silero-v6.2.0.bin: ggml load failed".into(),
        )
        .into();
        let IpcError::Internal { message } = e else {
            panic!("expected Internal");
        };
        assert_eq!(message, "vad model error");
        assert!(!message.contains("/Users/fran"));
        assert!(!message.contains(".bin"));
    }

    #[test]
    fn vad_io_does_not_leak_path() {
        let io = std::io::Error::other("/Users/fran/secret/path.wav: permission denied");
        let e: IpcError = VadError::Io(io).into();
        let IpcError::Internal { message } = e else {
            panic!("expected Internal");
        };
        assert_eq!(message, "vad io error");
        assert!(!message.contains("/Users/fran"));
    }
}
