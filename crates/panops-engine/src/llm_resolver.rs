//! Resolve which `LlmProvider` impl the engine uses at startup.
//!
//! On macOS, if `PANOPS_LLM_SIDECAR_BIN` is set AND that path is an
//! executable file, probe the FoundationModels sidecar (`panops_mac::FoundationLlm`).
//! If `SystemLanguageModel.availability` reports available, use it;
//! otherwise fall back to `GenaiLlm` (Ollama / provider env auto-detect).
//!
//! Slice 15 design:
//! `docs/superpowers/specs/2026-06-06-slice-15-foundationmodels-llm-sidecar-design.md`.

use std::sync::Arc;

use panops_core::llm::LlmProvider;
use panops_portable::genai_llm::GenaiLlm;

/// Resolve the LLM adapter as an `Arc<dyn LlmProvider + Send + Sync>`.
///
/// `sidecar_path` is intentionally an explicit argument instead of a
/// hidden read inside this function so `main.rs` owns the dev/CI-only
/// `PANOPS_LLM_SIDECAR_BIN` gate and production bundle resolution can
/// land later without changing the provider-selection policy.
pub fn pick_llm(
    handle: tokio::runtime::Handle,
    sidecar_path: Option<std::path::PathBuf>,
) -> Arc<dyn LlmProvider + Send + Sync> {
    #[cfg(not(target_os = "macos"))]
    let _ = sidecar_path;

    #[cfg(target_os = "macos")]
    {
        let resolved = sidecar_path
            .and_then(crate::sidecar_binary::executable_file)
            .or_else(|| crate::sidecar_binary::sibling_of_engine("panops-llm-mac"));
        if let Some(sidecar) = resolved {
            let llm = panops_mac::FoundationLlm::new(sidecar.clone());
            match llm.probe() {
                Ok(probe) if probe.available => {
                    tracing::info!(
                        sidecar = %sidecar.display(),
                        "selecting FoundationModels LLM sidecar"
                    );
                    return Arc::new(llm);
                }
                Ok(probe) => {
                    tracing::info!(
                        sidecar = %sidecar.display(),
                        reason = probe.reason.as_deref().unwrap_or("unavailable"),
                        "FoundationModels unavailable; falling back to GenaiLlm"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        sidecar = %sidecar.display(),
                        error = %e,
                        "FoundationModels sidecar probe failed; falling back to GenaiLlm"
                    );
                }
            }
        }
    }
    Arc::new(genai_with_handle(handle))
}

/// Read the dev/CI-only `PANOPS_LLM_SIDECAR_BIN` gate. The returned
/// path is not validated here; [`pick_llm`] canonicalizes + exec-bit
/// checks before spawning so tests can inject non-existent paths and
/// observe fallback.
pub fn sidecar_binary_from_env() -> Option<std::path::PathBuf> {
    std::env::var_os("PANOPS_LLM_SIDECAR_BIN").map(std::path::PathBuf::from)
}

fn genai_with_handle(handle: tokio::runtime::Handle) -> GenaiLlm {
    let model = if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        "claude-haiku-4-5-20251001"
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        "gpt-4o-mini"
    } else if std::env::var("OLLAMA_HOST").is_ok() {
        "gemma3:4b"
    } else {
        // Last-resort default. Matches `GenaiLlm::auto` so the IPC
        // server is no more restrictive than the CLI's auto mode.
        "gemma3:4b"
    };
    GenaiLlm::with_handle(model, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genai_default_model_is_constructible_without_env() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _llm = genai_with_handle(rt.handle().clone());
    }
}
