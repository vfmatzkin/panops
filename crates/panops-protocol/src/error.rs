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
