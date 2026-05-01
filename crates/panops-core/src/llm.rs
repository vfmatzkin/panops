//! `LlmProvider` port. Low-level: one `complete` call per LLM round-trip.
//!
//! Implementations live in adapter crates (real LLM providers) and in
//! `panops-core::conformance::fakes` (test fakes). Higher-level pipeline
//! orchestration that composes multiple `complete` calls is built on top of
//! this trait, not in it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub trait LlmProvider: Send + Sync {
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub user: String,
    /// JSON Schema describing the expected shape of a structured response.
    /// Adapters decide how to honor it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    pub temperature: f32,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmResponse {
    Text(String),
    Json(serde_json::Value),
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("network: {0}")]
    Network(String),
    #[error("invalid schema: expected {expected}, got {got}")]
    InvalidSchema { expected: String, got: String },
    #[error("empty response")]
    EmptyResponse,
    #[error("provider: {0}")]
    Provider(String),
    #[error("cancelled")]
    Cancelled,
}

/// Stable fingerprint of `(system, user)` used by `MockLlm` to key responses.
/// SHA-256 over `system.unwrap_or("") || "\n---\n" || user`. Schema/temperature
/// are intentionally NOT part of the fingerprint — tests register one canned
/// response per prompt and ignore tuning parameters.
pub fn prompt_fingerprint(system: Option<&str>, user: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(system.unwrap_or("").as_bytes());
    h.update(b"\n---\n");
    h.update(user.as_bytes());
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_request_round_trips_through_serde() {
        let req = LlmRequest {
            system: Some("you are a meeting note writer".into()),
            user: "summarise: hi".into(),
            schema: Some(serde_json::json!({"type": "object"})),
            temperature: 0.2,
            max_tokens: 1024,
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: LlmRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.user, req.user);
        assert_eq!(back.temperature, req.temperature);
        assert_eq!(back.max_tokens, req.max_tokens);
    }

    #[test]
    fn llm_error_display_includes_variant_name() {
        let e = LlmError::Network("timeout".into());
        assert!(format!("{e}").contains("network"));
    }

    #[test]
    fn llm_response_text_and_json_are_distinguishable() {
        let t = LlmResponse::Text("hi".into());
        let j = LlmResponse::Json(serde_json::json!({"k":"v"}));
        assert!(matches!(t, LlmResponse::Text(_)));
        assert!(matches!(j, LlmResponse::Json(_)));
    }
}
