use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
        verify_existing(name, dest)?;
        return Ok(dest.to_path_buf());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
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

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| AsrError::Model(format!("http client: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .map_err(|e| AsrError::Model(format!("download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AsrError::Model(format!("download HTTP {}", resp.status())));
    }

    // Stream into a sibling .partial file, hashing as we go. Atomic rename on success.
    let tmp = dest.with_extension("partial");
    let mut hasher = Sha256::new();
    let mut bytes_written: u64 = 0;
    {
        let mut file =
            fs::File::create(&tmp).map_err(|e| AsrError::Model(format!("create {tmp:?}: {e}")))?;
        let mut reader = resp;
        let mut buf = [0_u8; 64 * 1024];
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| AsrError::Model(format!("download read: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| AsrError::Model(format!("write {tmp:?}: {e}")))?;
            bytes_written += n as u64;
        }
        file.sync_all()
            .map_err(|e| AsrError::Model(format!("fsync {tmp:?}: {e}")))?;
    }

    if name == DEFAULT_MODEL && std::env::var("PANOPS_SKIP_MODEL_CHECKSUM").is_err() {
        let actual = format!("{:x}", hasher.finalize());
        if actual != DEFAULT_MODEL_SHA256 {
            let _ = fs::remove_file(&tmp);
            return Err(AsrError::Model(format!(
                "checksum mismatch for {name}: expected {DEFAULT_MODEL_SHA256}, got {actual}"
            )));
        }
    }

    fs::rename(&tmp, dest)
        .map_err(|e| AsrError::Model(format!("rename {tmp:?} -> {dest:?}: {e}")))?;
    eprintln!("Downloaded {bytes_written} bytes to {dest:?}");
    Ok(dest.to_path_buf())
}

fn verify_existing(name: &str, dest: &Path) -> Result<(), AsrError> {
    let meta = fs::metadata(dest).map_err(|e| AsrError::Model(format!("stat {dest:?}: {e}")))?;
    if !meta.is_file() {
        return Err(AsrError::Model(format!(
            "model path {dest:?} exists but is not a regular file"
        )));
    }
    if name == DEFAULT_MODEL && std::env::var("PANOPS_SKIP_MODEL_CHECKSUM").is_err() {
        let mut f =
            fs::File::open(dest).map_err(|e| AsrError::Model(format!("open {dest:?}: {e}")))?;
        let mut hasher = Sha256::new();
        let mut buf = [0_u8; 64 * 1024];
        loop {
            let n = f
                .read(&mut buf)
                .map_err(|e| AsrError::Model(format!("read {dest:?}: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let actual = format!("{:x}", hasher.finalize());
        if actual != DEFAULT_MODEL_SHA256 {
            return Err(AsrError::Model(format!(
                "checksum mismatch for cached {name} at {dest:?}: expected {DEFAULT_MODEL_SHA256}, got {actual}"
            )));
        }
    }
    Ok(())
}
