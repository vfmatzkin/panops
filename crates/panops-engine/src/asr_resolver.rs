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
        if let Some(sidecar) = sidecar_binary() {
            tracing::info!(
                sidecar = %sidecar.display(),
                "selecting WhisperKit ASR sidecar"
            );
            return Ok(Arc::new(panops_mac::WhisperKitAsr::new(sidecar)));
        }
    }
    let inner = WhisperRsAsr::new(model_path)?;
    Ok(Arc::new(inner))
}

#[cfg(target_os = "macos")]
fn sidecar_binary() -> Option<std::path::PathBuf> {
    let bin = std::env::var("PANOPS_ASR_SIDECAR_BIN").ok()?;
    let path = std::path::PathBuf::from(bin);
    let meta = std::fs::metadata(&path).ok()?;
    if meta.is_file() { Some(path) } else { None }
}
