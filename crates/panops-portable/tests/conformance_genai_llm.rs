//! Real-LLM conformance test, gated behind PANOPS_RUN_LLM_TESTS=1 and at
//! least one provider env (OLLAMA_HOST / ANTHROPIC_API_KEY / OPENAI_API_KEY).
//! CI does not run this.

use panops_core::conformance::llm::run_suite;
use panops_portable::genai_llm::GenaiLlm;

#[test]
fn real_genai_passes_conformance() {
    if std::env::var("PANOPS_RUN_LLM_TESTS").is_err() {
        eprintln!("skipping real_genai_passes_conformance: set PANOPS_RUN_LLM_TESTS=1");
        return;
    }
    let llm = GenaiLlm::auto().expect("no provider configured");
    run_suite(&llm);
}
