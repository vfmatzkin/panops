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
    use std::os::unix::fs::PermissionsExt;

    let bin = std::env::var("PANOPS_ASR_SIDECAR_BIN").ok()?;
    // Canonicalize before the metadata check so a symlink can't be
    // swapped between validation and `Command::spawn` (TOCTOU). The
    // resolved path is what we hand to `Command::new` below.
    let path = std::fs::canonicalize(bin).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    // Verify execute permission for owner/group/other (0o111). A regular
    // file without exec bits would `spawn`-fail at runtime; cleaner to
    // reject up front and fall back to whisper-rs.
    if meta.permissions().mode() & 0o111 == 0 {
        return None;
    }
    Some(path)
}
