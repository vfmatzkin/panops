use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use panops_core::asr::AsrError;
use sha2::{Digest, Sha256};

pub const DEFAULT_MODEL: &str = "ggml-tiny-q5_1.bin";
/// SHA-256 of `ggml-tiny-q5_1.bin` from Hugging Face. Verified 2026-04-30
/// against `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin`.
/// If HF re-uploads the file the hash changes; refresh via
/// `curl -sSfL <url> | shasum -a 256`.
pub const DEFAULT_MODEL_SHA256: &str =
    "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7";
pub const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin";

pub fn default_model_path() -> Result<PathBuf, AsrError> {
    if let Ok(p) = std::env::var("PANOPS_MODEL") {
        return Ok(PathBuf::from(p));
    }
    let dirs = ProjectDirs::from("dev", "panops", "panops")
        .ok_or_else(|| AsrError::Model("could not resolve project dirs".to_string()))?;
    Ok(dirs.data_dir().join("models").join(DEFAULT_MODEL))
}

pub fn ensure_model(name: &str, dest: &Path) -> Result<PathBuf, AsrError> {
    if dest.exists() {
        return Ok(dest.to_path_buf());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let url = match name {
        DEFAULT_MODEL => DEFAULT_MODEL_URL,
        other => {
            return Err(AsrError::Model(format!(
                "no download URL registered for model {other}"
            )));
        }
    };
    eprintln!("Downloading {name} (~31 MB) from {url}...");
    let resp = reqwest::blocking::get(url)
        .map_err(|e| AsrError::Model(format!("download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AsrError::Model(format!("download HTTP {}", resp.status())));
    }
    let mut bytes = Vec::with_capacity(32 * 1024 * 1024);
    let mut reader = resp;
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| AsrError::Model(format!("download read: {e}")))?;

    if name == DEFAULT_MODEL && std::env::var("PANOPS_SKIP_MODEL_CHECKSUM").is_err() {
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != DEFAULT_MODEL_SHA256 {
            return Err(AsrError::Model(format!(
                "checksum mismatch for {name}: expected {DEFAULT_MODEL_SHA256}, got {actual}"
            )));
        }
    }

    let mut f = std::fs::File::create(dest)
        .map_err(|e| AsrError::Model(format!("create {dest:?}: {e}")))?;
    f.write_all(&bytes)
        .map_err(|e| AsrError::Model(format!("write {dest:?}: {e}")))?;
    eprintln!("Downloaded {} bytes to {dest:?}", bytes.len());
    Ok(dest.to_path_buf())
}
