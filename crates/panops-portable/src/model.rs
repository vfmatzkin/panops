use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use directories::ProjectDirs;
use panops_core::asr::AsrError;
use sha2::{Digest, Sha256};

pub struct ModelInfo {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub approx_size_mb: u32,
}

pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        name: "ggml-tiny-q5_1",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin",
        sha256: "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
        approx_size_mb: 31,
    },
    ModelInfo {
        name: "ggml-base-q5_1",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin",
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        approx_size_mb: 57,
    },
    ModelInfo {
        name: "ggml-large-v3-turbo-q5_0",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        approx_size_mb: 547,
    },
];

pub const DIAR_MODELS: &[ModelInfo] = &[
    ModelInfo {
        name: "sherpa-onnx-pyannote-segmentation-3-0",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
        sha256: "24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488",
        approx_size_mb: 7,
    },
    ModelInfo {
        name: "3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx",
        sha256: "1a331345f04805badbb495c775a6ddffcdd1a732567d5ec8b3d5749e3c7a5e4b",
        approx_size_mb: 38,
    },
];

pub const DEFAULT_MODEL_NAME: &str = "ggml-large-v3-turbo-q5_0";

fn data_dir() -> Result<PathBuf, AsrError> {
    let dirs = ProjectDirs::from("dev", "panops", "panops")
        .ok_or_else(|| AsrError::Model("could not resolve project dirs".to_string()))?;
    Ok(dirs.data_dir().join("models"))
}

pub fn default_model_path() -> Result<PathBuf, AsrError> {
    if let Ok(p) = std::env::var("PANOPS_MODEL") {
        return Ok(PathBuf::from(p));
    }
    Ok(data_dir()?.join(format!("{DEFAULT_MODEL_NAME}.bin")))
}

pub fn default_diar_seg_path() -> Result<PathBuf, AsrError> {
    if let Ok(p) = std::env::var("PANOPS_DIAR_SEG") {
        return Ok(PathBuf::from(p));
    }
    Ok(data_dir()?
        .join("sherpa-onnx-pyannote-segmentation-3-0")
        .join("model.onnx"))
}

pub fn default_diar_emb_path() -> Result<PathBuf, AsrError> {
    if let Ok(p) = std::env::var("PANOPS_DIAR_EMB") {
        return Ok(PathBuf::from(p));
    }
    Ok(data_dir()?.join("3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"))
}

fn http_client() -> Result<reqwest::blocking::Client, AsrError> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| AsrError::Model(format!("http client: {e}")))
}

fn lookup_model(name: &str) -> Result<&'static ModelInfo, AsrError> {
    MODELS
        .iter()
        .chain(DIAR_MODELS.iter())
        .find(|m| m.name == name)
        .ok_or_else(|| AsrError::Model(format!("no registered model named {name}")))
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), AsrError> {
    let mut f = fs::File::open(path).map_err(|e| AsrError::Model(format!("open {path:?}: {e}")))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| AsrError::Model(format!("read {path:?}: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(AsrError::Model(format!(
            "checksum mismatch at {path:?}: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

fn download(client: &reqwest::blocking::Client, url: &str, dest: &Path) -> Result<u64, AsrError> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let resp = client
        .get(url)
        .send()
        .map_err(|e| AsrError::Model(format!("download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AsrError::Model(format!("download HTTP {}", resp.status())));
    }
    let tmp = dest.with_extension("partial");
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
            file.write_all(&buf[..n])
                .map_err(|e| AsrError::Model(format!("write {tmp:?}: {e}")))?;
            bytes_written += n as u64;
        }
        file.sync_all()
            .map_err(|e| AsrError::Model(format!("fsync {tmp:?}: {e}")))?;
    }
    fs::rename(&tmp, dest)
        .map_err(|e| AsrError::Model(format!("rename {tmp:?} -> {dest:?}: {e}")))?;
    Ok(bytes_written)
}

/// Ensure a registered model exists at `dest`. Verifies sha256 against the
/// registered hash for `name`. Idempotent. Used for both Whisper `.bin`
/// files and bare `.onnx` files (not for tarballs — see `ensure_diar_models`).
///
/// Behavior on existing files:
/// - If `PANOPS_MODEL` env is set: trust the user-provided file, skip checksum
///   (the user explicitly chose this path, possibly pointing at a different
///   registered model than `name`).
/// - If `PANOPS_SKIP_MODEL_CHECKSUM` env is set: skip checksum.
/// - Otherwise: verify against the registered hash.
pub fn ensure_model(name: &str, dest: &Path) -> Result<PathBuf, AsrError> {
    let info = lookup_model(name)?;
    if dest.exists() {
        let user_override = std::env::var("PANOPS_MODEL").is_ok();
        let skip_checksum = std::env::var("PANOPS_SKIP_MODEL_CHECKSUM").is_ok();
        if !user_override && !skip_checksum {
            verify_sha256(dest, info.sha256)?;
        }
        return Ok(dest.to_path_buf());
    }
    eprintln!(
        "Downloading {} (~{} MB) from {}...",
        info.name, info.approx_size_mb, info.url
    );
    let client = http_client()?;
    let n = download(&client, info.url, dest)?;
    verify_sha256(dest, info.sha256)?;
    eprintln!("Downloaded {n} bytes to {dest:?}");
    Ok(dest.to_path_buf())
}

/// Ensure both diarization ONNX models exist on disk. Returns
/// (segmentation_model_path, embedding_model_path). Honors
/// PANOPS_DIAR_SEG / PANOPS_DIAR_EMB env overrides. Handles the
/// segmentation tarball download + extraction transparently.
pub fn ensure_diar_models() -> Result<(PathBuf, PathBuf), AsrError> {
    let seg = default_diar_seg_path()?;
    let emb = default_diar_emb_path()?;

    // Embedding model is a bare .onnx; standard ensure_model.
    let emb_info = lookup_model("3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k")?;
    if !emb.exists() {
        if let Some(parent) = emb.parent() {
            fs::create_dir_all(parent)?;
        }
        eprintln!(
            "Downloading {} (~{} MB)...",
            emb_info.name, emb_info.approx_size_mb
        );
        let client = http_client()?;
        download(&client, emb_info.url, &emb)?;
    }
    let skip_checksum = std::env::var("PANOPS_SKIP_MODEL_CHECKSUM").is_ok();
    let emb_user_override = std::env::var("PANOPS_DIAR_EMB").is_ok();
    if !skip_checksum && !emb_user_override {
        verify_sha256(&emb, emb_info.sha256)?;
    }

    // Segmentation model is in a tar.bz2; download + extract.
    if !seg.exists() {
        let seg_info = lookup_model("sherpa-onnx-pyannote-segmentation-3-0")?;
        let dir = data_dir()?;
        fs::create_dir_all(&dir)?;
        let tar_path = dir.join("sherpa-onnx-pyannote-segmentation-3-0.tar.bz2");
        if !tar_path.exists() {
            eprintln!(
                "Downloading {} (~{} MB)...",
                seg_info.name, seg_info.approx_size_mb
            );
            let client = http_client()?;
            download(&client, seg_info.url, &tar_path)?;
        }
        if std::env::var("PANOPS_SKIP_MODEL_CHECKSUM").is_err() {
            verify_sha256(&tar_path, seg_info.sha256)?;
        }
        let f = fs::File::open(&tar_path)
            .map_err(|e| AsrError::Model(format!("open {tar_path:?}: {e}")))?;
        let decoder = bzip2::read::BzDecoder::new(f);
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(&dir)
            .map_err(|e| AsrError::Model(format!("untar {tar_path:?}: {e}")))?;
        if !seg.exists() {
            return Err(AsrError::Model(format!(
                "expected {seg:?} after extracting tarball; archive layout changed?"
            )));
        }
    }

    Ok((seg, emb))
}
