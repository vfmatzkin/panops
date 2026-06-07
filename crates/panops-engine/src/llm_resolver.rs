//! Resolve which `LlmProvider` impl the engine uses at startup.
//!
//! On macOS, if `PANOPS_LLM_SIDECAR_BIN` is set AND that path is an
//! executable file, probe the FoundationModels sidecar (`panops_mac::FoundationLlm`).
//! If `SystemLanguageModel.availability` reports available, use it;
//! otherwise fall back to local Ollama via `GenaiLlm`. Cloud auto-detect is
//! reserved for the explicit CLI `--llm-provider auto` path.
//!
//! Slice 15 design:
//! `docs/superpowers/specs/2026-06-06-slice-15-foundationmodels-llm-sidecar-design.md`.

use std::sync::Arc;

use panops_core::llm::LlmProvider;
use panops_portable::genai_llm::GenaiLlm;
use panops_protocol::LlmInfo;

/// Resolve the LLM adapter and its wire-facing description.
///
/// `sidecar_path` is intentionally an explicit argument instead of a
/// hidden read inside this function so `main.rs` owns the dev/CI-only
/// `PANOPS_LLM_SIDECAR_BIN` gate and production bundle resolution can
/// land later without changing the provider-selection policy.
pub fn pick_llm(
    handle: tokio::runtime::Handle,
    sidecar_path: Option<std::path::PathBuf>,
) -> (Arc<dyn LlmProvider + Send + Sync>, LlmInfo) {
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
                    return (Arc::new(llm), LlmInfo::apple_foundation());
                }
                Ok(probe) => {
                    tracing::info!(
                        sidecar = %sidecar.display(),
                        reason = probe.reason.as_deref().unwrap_or("unavailable"),
                        "FoundationModels unavailable; falling back to local Ollama"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        sidecar = %sidecar.display(),
                        error = %e,
                        "FoundationModels sidecar probe failed; falling back to local Ollama"
                    );
                }
            }
        }
    }
    (Arc::new(genai_with_handle(handle)), LlmInfo::local_ollama())
}

/// Read the dev/CI-only `PANOPS_LLM_SIDECAR_BIN` gate. The returned
/// path is not validated here; [`pick_llm`] canonicalizes + exec-bit
/// checks before spawning so tests can inject non-existent paths and
/// observe fallback.
pub fn sidecar_binary_from_env() -> Option<std::path::PathBuf> {
    std::env::var_os("PANOPS_LLM_SIDECAR_BIN").map(std::path::PathBuf::from)
}

/// Build the FoundationModels sidecar adapter for an **explicit**
/// `--llm-provider foundation` request. Unlike [`pick_llm`], this never
/// falls back: a missing or non-executable sidecar is a hard error so
/// the operator knows the on-device path didn't actually run rather than
/// silently getting Ollama/cloud notes. Construction mirrors `pick_llm`
/// (`FoundationLlm::new` over an exec-validated path).
#[cfg(target_os = "macos")]
pub fn foundation_llm_explicit(
    sidecar_path: Option<std::path::PathBuf>,
) -> Result<panops_mac::FoundationLlm, String> {
    let raw = sidecar_path.ok_or_else(|| {
        "--llm-provider foundation requires PANOPS_LLM_SIDECAR_BIN to point at an \
         executable panops-llm-mac sidecar; it is not set"
            .to_string()
    })?;
    let resolved = crate::sidecar_binary::executable_file(raw.clone()).ok_or_else(|| {
        format!(
            "--llm-provider foundation: PANOPS_LLM_SIDECAR_BIN ({}) is not an executable file",
            raw.display()
        )
    })?;
    Ok(panops_mac::FoundationLlm::new(resolved))
}

fn genai_with_handle(handle: tokio::runtime::Handle) -> GenaiLlm {
    let model = "gemma3:4b";
    tracing::info!(
        model = model,
        "notes LLM: using local ollama (set --llm-provider auto for cloud)"
    );
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
