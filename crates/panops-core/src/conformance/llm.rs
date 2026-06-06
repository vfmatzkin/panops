//! Conformance harness for `LlmProvider`. Adapter test crates call
//! `run_suite(&adapter)` after registering the canned prompts the harness
//! probes for.
//!
//! Error-path behavior lives in `run_error_suite` / `assert_empty_response`.
//! A live LLM can't be made to fail on command, so the error contract is
//! driven by fakes rigged to fail (`MockLlm::with_error_for`, `FailingLlm`)
//! rather than the provider-under-test — real adapters run only `run_suite`.

use crate::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};

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

/// Prompt `run_error_suite` / `assert_empty_response` send to `complete`.
/// Adapters that rig failures per-prompt (e.g. `MockLlm::with_error_for`)
/// must arrange the failure for this exact `(system = None, user)` pair;
/// adapters that fail unconditionally (`FailingLlm`) ignore it.
pub const ERROR_PROBE_USER: &str = "conformance: force backend failure";

/// Error-path conformance: a provider whose backend fails MUST return
/// `Err(LlmError)` from `complete` and MUST NOT panic. Drive with any fake
/// rigged to fail (`FailingLlm`, or `MockLlm::with_error_for(None,
/// ERROR_PROBE_USER, ...)`). Live adapters can't be forced to fail, so they
/// run only `run_suite`.
pub fn run_error_suite<L: LlmProvider>(failing: &L) {
    backend_failure_returns_err_not_panic(failing);
}

/// Empty-backend conformance: a provider whose backend returned a blank
/// completion MUST surface `LlmError::EmptyResponse` (the documented behavior
/// — see `panops-portable`'s `GenaiLlm`, which maps an empty `first_text()`
/// the same way). Drive with a fake modelling an empty backend
/// (`FailingLlm::empty`).
pub fn assert_empty_response<L: LlmProvider>(empty_provider: &L) {
    match empty_provider.complete(error_probe_request()) {
        Err(LlmError::EmptyResponse) => {}
        other => {
            panic!("expected Err(LlmError::EmptyResponse) for an empty backend, got {other:?}")
        }
    }
}

fn backend_failure_returns_err_not_panic<L: LlmProvider>(failing: &L) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        failing.complete(error_probe_request())
    }));
    match outcome {
        Err(_) => panic!("complete panicked instead of returning Err(LlmError)"),
        Ok(Ok(_)) => panic!("error-suite provider returned Ok; expected Err(LlmError)"),
        Ok(Err(_)) => {}
    }
}

fn error_probe_request() -> LlmRequest {
    LlmRequest {
        system: None,
        user: ERROR_PROBE_USER.into(),
        schema: None,
        temperature: 0.0,
        max_tokens: 16,
    }
}
