//! Conformance harness for `LlmProvider`. Adapter test crates call
//! `run_suite(&adapter)` after registering the canned prompts the harness
//! probes for.

use crate::llm::{LlmProvider, LlmRequest, LlmResponse};

pub fn run_suite<L: LlmProvider>(provider: &L) {
    text_response_round_trip(provider);
    json_response_round_trip(provider);
}

fn text_response_round_trip<L: LlmProvider>(provider: &L) {
    let req = LlmRequest {
        system: None,
        user: "say hi".into(),
        schema: None,
        temperature: 0.2,
        max_tokens: 16,
    };
    match provider.complete(req).expect("complete failed") {
        LlmResponse::Text(s) => assert!(!s.is_empty(), "empty text response"),
        LlmResponse::Json(_) => panic!("expected text, got json"),
    }
}

fn json_response_round_trip<L: LlmProvider>(provider: &L) {
    let req = LlmRequest {
        system: None,
        user: "json please".into(),
        schema: Some(serde_json::json!({"type": "object"})),
        temperature: 0.0,
        max_tokens: 64,
    };
    match provider.complete(req).expect("complete failed") {
        LlmResponse::Json(v) => assert!(v.is_object(), "not an object"),
        LlmResponse::Text(_) => panic!("expected json, got text"),
    }
}
