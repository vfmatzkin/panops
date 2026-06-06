//! Integration test: validates that `MockLlm` satisfies the `LlmProvider`
//! conformance harness — happy paths plus the error-path contract every
//! adapter must honor.

use panops_core::conformance::fakes::{FailingLlm, MockLlm};
use panops_core::conformance::llm::{
    ERROR_PROBE_USER, assert_empty_response, run_error_suite, run_suite,
};

#[test]
fn mock_llm_passes_conformance() {
    let mock = MockLlm::default()
        .with_response_for(None, "say hi", panops_core::LlmResponse::Text("hi".into()))
        .with_response_for(
            None,
            "json please",
            panops_core::LlmResponse::Json(serde_json::json!({"ok": true})),
        );
    run_suite(&mock);
}

#[test]
fn llm_adapters_honor_error_contract() {
    // MockLlm is a real adapter (the CI one): its registered-error path must
    // return Err without panicking.
    let failing_mock = MockLlm::default().with_error_for(None, ERROR_PROBE_USER, "backend down");
    run_error_suite(&failing_mock);

    // The reference FailingLlm fake honors the same universal contract...
    run_error_suite(&FailingLlm::provider("backend down"));

    // ...plus the empty-backend -> EmptyResponse contract MockLlm doesn't model.
    assert_empty_response(&FailingLlm::empty());
}
