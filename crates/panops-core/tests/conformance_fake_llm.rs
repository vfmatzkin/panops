//! Integration test: validates that `MockLlm` satisfies the `LlmProvider`
//! conformance harness. Uses only public API + the conformance module.

use panops_core::conformance::fakes::MockLlm;
use panops_core::conformance::llm::run_suite;

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
