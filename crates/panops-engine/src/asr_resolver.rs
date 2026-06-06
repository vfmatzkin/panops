//! Resolve which `AsrProvider` impl the engine uses at startup.
//!
//! On macOS, if `PANOPS_ASR_SIDECAR_BIN` is set AND that path is an
//! executable file, use the WhisperKit sidecar (`panops_mac::WhisperKitAsr`)
//! for CoreML+Metal-accelerated transcription. Otherwise fall back to
//! `WhisperRsAsr` (whisper.cpp CPU/BLAS).
//!
//! Slice 10 design:
//! `docs/superpowers/specs/2026-05-12-slice-10-whisperkit-asr-sidecar-design.md`.

use std::sync::Arc;

use panops_core::asr::{AsrError, AsrProvider};
use panops_portable::WhisperRsAsr;

/// Resolve the ASR adapter as an `Arc<dyn AsrProvider + Send + Sync>`,
/// for storage in `HeavyAdapters`.
pub fn pick_asr(
    model_path: std::path::PathBuf,
) -> Result<Arc<dyn AsrProvider + Send + Sync>, AsrError> {
    #[cfg(target_os = "macos")]
    {
        let sidecar = std::env::var_os("PANOPS_ASR_SIDECAR_BIN")
            .and_then(|v| crate::sidecar_binary::executable_file(std::path::PathBuf::from(v)))
            .or_else(|| crate::sidecar_binary::sibling_of_engine("panops-asr-mac"));
        if let Some(sidecar) = sidecar {
            tracing::info!(sidecar = %sidecar.display(), "selecting WhisperKit ASR sidecar");
            return Ok(Arc::new(panops_mac::WhisperKitAsr::new(sidecar)));
        }
    }
    let inner = WhisperRsAsr::new(model_path)?;
    Ok(Arc::new(inner))
}
