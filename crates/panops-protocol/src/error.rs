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
    use panops_core::llm::LlmError;
    use panops_core::notes::error::NotesError;

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
}

#[cfg(all(test, feature = "domain-conversions"))]
mod from_domain_tests {
    use super::IpcError;
    use panops_core::asr::AsrError;
    use panops_core::diar::DiarError;
    use panops_core::llm::LlmError;
    use panops_core::notes::error::NotesError;
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
}
