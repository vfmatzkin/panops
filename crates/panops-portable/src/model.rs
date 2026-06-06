use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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

pub const VAD_MODELS: &[ModelInfo] = &[ModelInfo {
    name: "ggml-silero-v6.2.0",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin",
    // SHA256 captured 2026-05-09 by `shasum -a 256` after manual download.
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    approx_size_mb: 1,
}];

pub const DEFAULT_MODEL_NAME: &str = "ggml-large-v3-turbo-q5_0";

pub const DEFAULT_VAD_MODEL_NAME: &str = "ggml-silero-v6.2.0";

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

pub fn default_vad_model_path() -> Result<PathBuf, AsrError> {
    if let Ok(p) = std::env::var("PANOPS_VAD_MODEL") {
        return Ok(PathBuf::from(p));
    }
    Ok(data_dir()?.join(format!("{DEFAULT_VAD_MODEL_NAME}.bin")))
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
        .chain(VAD_MODELS.iter())
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

const DOWNLOAD_PROGRESS_INTERVAL: Duration = Duration::from_secs(2);
/// Tolerance over the server's Content-Length before aborting an over-large
/// download (a compromised/MITM host could otherwise fill the disk before the
/// post-download checksum runs).
const DOWNLOAD_SIZE_GRACE: u64 = 1024 * 1024; // 1 MiB
/// Hard ceiling when the server sends no Content-Length — bounds an unbounded
/// stream. Comfortably above the largest registered model.
const MAX_MODEL_DOWNLOAD_BYTES: u64 = 6 * 1024 * 1024 * 1024; // 6 GiB

/// Byte limit for an in-progress download. Always clamped to the hard ceiling,
/// so an attacker-controlled `Content-Length` (even `u64::MAX`) can't raise it.
fn download_size_limit(total_bytes: Option<u64>) -> u64 {
    match total_bytes {
        Some(t) => t
            .saturating_add(DOWNLOAD_SIZE_GRACE)
            .min(MAX_MODEL_DOWNLOAD_BYTES),
        None => MAX_MODEL_DOWNLOAD_BYTES,
    }
}

fn percent_complete(done: u64, total: u64) -> Option<u8> {
    if total == 0 {
        return None;
    }
    let pct = ((done.min(total) as f64 / total as f64) * 100.0).round() as u8;
    Some(pct)
}

fn bytes_per_second(bytes: u64, elapsed: Duration) -> u64 {
    if elapsed.is_zero() {
        return 0;
    }
    (bytes as f64 / elapsed.as_secs_f64()).round() as u64
}

fn eta_secs(done: u64, total: u64, bytes_per_sec: u64) -> Option<u64> {
    if done >= total {
        return Some(0);
    }
    if total == 0 || bytes_per_sec == 0 {
        return None;
    }
    let remaining = total - done;
    Some(remaining / bytes_per_sec + u64::from(remaining % bytes_per_sec != 0))
}

fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let bytes = bytes as f64;
    if bytes < KIB {
        format!("{bytes:.0} B")
    } else if bytes < MIB {
        format!("{:.1} KiB", bytes / KIB)
    } else if bytes < GIB {
        format!("{:.1} MiB", bytes / MIB)
    } else {
        format!("{:.1} GiB", bytes / GIB)
    }
}

fn human_rate(bytes_per_sec: u64) -> String {
    format!("{}/s", human_bytes(bytes_per_sec))
}

fn format_duration_secs(seconds: u64) -> String {
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    if minutes == 0 {
        format!("{seconds}s")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

fn format_download_progress(
    bytes_downloaded: u64,
    total_bytes: Option<u64>,
    bytes_per_sec: u64,
) -> String {
    let downloaded = human_bytes(bytes_downloaded);
    let rate = human_rate(bytes_per_sec);
    match total_bytes {
        Some(total) => {
            let total_display = human_bytes(total);
            let percent = percent_complete(bytes_downloaded, total)
                .map(|pct| format!("{pct}%"))
                .unwrap_or_else(|| "unknown".to_string());
            let eta = eta_secs(bytes_downloaded, total, bytes_per_sec)
                .map(format_duration_secs)
                .unwrap_or_else(|| "unknown".to_string());
            format!("downloaded {downloaded} / {total_display} ({percent}) at {rate}, eta {eta}")
        }
        None => format!("downloaded {downloaded} (total unknown) at {rate}"),
    }
}

fn emit_download_progress(
    bytes_downloaded: u64,
    total_bytes: Option<u64>,
    bytes_since_last: u64,
    elapsed_since_last: Duration,
    complete: bool,
) {
    let bytes_per_sec = bytes_per_second(bytes_since_last, elapsed_since_last);
    let percent = total_bytes.and_then(|total| percent_complete(bytes_downloaded, total));
    let eta_secs = total_bytes.and_then(|total| eta_secs(bytes_downloaded, total, bytes_per_sec));
    let summary = format_download_progress(bytes_downloaded, total_bytes, bytes_per_sec);
    let msg = if complete {
        "model download progress complete"
    } else {
        "model download progress"
    };
    tracing::info!(
        bytes = bytes_downloaded,
        total_bytes = ?total_bytes,
        percent = ?percent,
        bytes_per_sec,
        rate = %human_rate(bytes_per_sec),
        eta_secs = ?eta_secs,
        progress = %summary,
        "{msg}"
    );
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
    let total_bytes = resp.content_length();
    // Fail fast on a known-oversized Content-Length (don't stream gigabytes
    // first); the in-loop guard handles servers that under-report then overrun.
    if let Some(total) = total_bytes {
        if total > MAX_MODEL_DOWNLOAD_BYTES {
            return Err(AsrError::Model(format!(
                "server Content-Length {total} exceeds max {MAX_MODEL_DOWNLOAD_BYTES} bytes; aborting"
            )));
        }
    }
    let tmp = dest.with_extension("partial");
    let mut bytes_written: u64 = 0;
    {
        let mut file =
            fs::File::create(&tmp).map_err(|e| AsrError::Model(format!("create {tmp:?}: {e}")))?;
        let mut reader = resp;
        let download_started_at = Instant::now();
        let mut last_progress_at = download_started_at;
        let mut last_progress_bytes = 0_u64;
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
            // Bound the download: cap at Content-Length + grace, or a hard
            // ceiling when the size is unknown, so a compromised/MITM host
            // can't fill the disk before the post-download checksum catches it.
            let limit = download_size_limit(total_bytes);
            if bytes_written > limit {
                if let Err(e) = fs::remove_file(&tmp) {
                    tracing::warn!(error = %e, tmp = ?tmp, "failed to remove oversized partial download");
                }
                return Err(AsrError::Model(format!(
                    "download exceeded size limit ({bytes_written} > {limit} bytes); aborting"
                )));
            }
            let now = Instant::now();
            // saturating: never panic if the monotonic clock appears to go
            // backwards (VM/container clock adjustments).
            let elapsed = now.saturating_duration_since(last_progress_at);
            if elapsed >= DOWNLOAD_PROGRESS_INTERVAL {
                let bytes_since_last = bytes_written - last_progress_bytes;
                emit_download_progress(
                    bytes_written,
                    total_bytes,
                    bytes_since_last,
                    elapsed,
                    false,
                );
                last_progress_at = now;
                last_progress_bytes = bytes_written;
            }
        }
        // Final line reports the OVERALL AVERAGE rate (cumulative bytes ÷ total
        // elapsed), the meaningful completion metric — distinct from the periodic
        // interval rates. saturating to avoid a non-monotonic-clock panic.
        emit_download_progress(
            bytes_written,
            total_bytes,
            bytes_written,
            Instant::now().saturating_duration_since(download_started_at),
            true,
        );
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
    tracing::info!(
        name = info.name,
        approx_mb = info.approx_size_mb,
        url = info.url,
        "downloading model"
    );
    let client = http_client()?;
    let n = download(&client, info.url, dest)?;
    if let Err(e) = verify_sha256(dest, info.sha256) {
        // Self-heal: a checksum mismatch means we have a poisoned cache.
        // Drop the file so a subsequent run re-downloads instead of looping.
        let _ = fs::remove_file(dest);
        return Err(e);
    }
    tracing::info!(bytes = n, dest = ?dest, "model download complete");
    Ok(dest.to_path_buf())
}

/// Ensure both diarization ONNX models exist on disk. Returns
/// (segmentation_model_path, embedding_model_path). Honors
/// PANOPS_DIAR_SEG / PANOPS_DIAR_EMB env overrides. Handles the
/// segmentation tarball download + extraction transparently.
pub fn ensure_diar_models() -> Result<(PathBuf, PathBuf), AsrError> {
    let seg = default_diar_seg_path()?;
    let emb = default_diar_emb_path()?;

    // Embedding model is a bare .onnx.
    let emb_info = lookup_model("3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k")?;
    let emb_existed_before = emb.exists();
    if !emb_existed_before {
        if let Some(parent) = emb.parent() {
            fs::create_dir_all(parent)?;
        }
        tracing::info!(
            name = emb_info.name,
            approx_mb = emb_info.approx_size_mb,
            "downloading diar embedding model"
        );
        let client = http_client()?;
        download(&client, emb_info.url, &emb)?;
    }
    let skip_checksum = std::env::var("PANOPS_SKIP_MODEL_CHECKSUM").is_ok();
    let emb_user_override = std::env::var("PANOPS_DIAR_EMB").is_ok();
    // Always verify what we just downloaded; only honor the override skip
    // for files that pre-existed (the user-trusted case).
    let verify_emb = !skip_checksum && (!emb_existed_before || !emb_user_override);
    if verify_emb {
        if let Err(e) = verify_sha256(&emb, emb_info.sha256) {
            if !emb_existed_before {
                let _ = fs::remove_file(&emb);
            }
            return Err(e);
        }
    }

    // Segmentation model is in a tar.bz2.
    if !seg.exists() {
        // If the user explicitly pointed PANOPS_DIAR_SEG at a path, we don't
        // know where to extract on their behalf — error early with a clear
        // message instead of silently extracting into the default data dir.
        if std::env::var("PANOPS_DIAR_SEG").is_ok() {
            return Err(AsrError::Model(format!(
                "PANOPS_DIAR_SEG points to {seg:?} but the file does not exist; pre-extract the segmentation tarball there or unset the env var to use the default data dir"
            )));
        }
        let seg_info = lookup_model("sherpa-onnx-pyannote-segmentation-3-0")?;
        let dir = data_dir()?;
        fs::create_dir_all(&dir)?;
        let tar_path = dir.join("sherpa-onnx-pyannote-segmentation-3-0.tar.bz2");
        let tar_existed_before = tar_path.exists();
        if !tar_existed_before {
            tracing::info!(
                name = seg_info.name,
                approx_mb = seg_info.approx_size_mb,
                "downloading diar segmentation model"
            );
            let client = http_client()?;
            download(&client, seg_info.url, &tar_path)?;
        }
        if !skip_checksum {
            if let Err(e) = verify_sha256(&tar_path, seg_info.sha256) {
                if !tar_existed_before {
                    let _ = fs::remove_file(&tar_path);
                }
                return Err(e);
            }
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

/// Ensure the VAD model exists at `dest`. Verifies sha256 against the
/// registered hash. Idempotent. The VAD model is a single `.bin` file
/// (no tarball, no extraction).
///
/// Behavior on existing files:
/// - If `PANOPS_VAD_MODEL` env is set: trust the user-provided file, skip checksum.
/// - If `PANOPS_SKIP_MODEL_CHECKSUM` env is set: skip checksum.
/// - Otherwise: verify against the registered hash.
pub fn ensure_vad_model(dest: &Path) -> Result<PathBuf, AsrError> {
    let info = &VAD_MODELS[0];
    if dest.exists() {
        let user_override = std::env::var("PANOPS_VAD_MODEL").is_ok();
        let skip_checksum = std::env::var("PANOPS_SKIP_MODEL_CHECKSUM").is_ok();
        if !user_override && !skip_checksum {
            verify_sha256(dest, info.sha256)?;
        }
        return Ok(dest.to_path_buf());
    }
    tracing::info!(
        name = info.name,
        approx_mb = info.approx_size_mb,
        url = info.url,
        "downloading vad model"
    );
    let client = http_client()?;
    let n = download(&client, info.url, dest)?;
    if let Err(e) = verify_sha256(dest, info.sha256) {
        let _ = fs::remove_file(dest);
        return Err(e);
    }
    tracing::info!(bytes = n, dest = ?dest, "vad model download complete");
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_complete_rounds_to_nearest_integer() {
        assert_eq!(percent_complete(1, 3), Some(33));
        assert_eq!(percent_complete(2, 3), Some(67));
        assert_eq!(percent_complete(999, 1000), Some(100));
        assert_eq!(percent_complete(120, 100), Some(100));
        assert_eq!(percent_complete(1, 0), None);
    }

    #[test]
    fn human_bytes_formats_each_unit_branch() {
        assert_eq!(human_bytes(500), "500 B");
        assert_eq!(human_bytes(2048), "2.0 KiB");
        assert_eq!(human_bytes(1_572_864), "1.5 MiB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn download_size_limit_always_clamps_to_hard_ceiling() {
        assert_eq!(download_size_limit(None), MAX_MODEL_DOWNLOAD_BYTES);
        assert_eq!(download_size_limit(Some(1000)), 1000 + DOWNLOAD_SIZE_GRACE);
        // Attacker-controlled huge / u64::MAX must NOT raise the limit.
        assert_eq!(
            download_size_limit(Some(u64::MAX)),
            MAX_MODEL_DOWNLOAD_BYTES
        );
        assert_eq!(
            download_size_limit(Some(MAX_MODEL_DOWNLOAD_BYTES * 2)),
            MAX_MODEL_DOWNLOAD_BYTES
        );
    }

    #[test]
    fn format_duration_secs_covers_zero_sub_minute_and_multi_minute() {
        assert_eq!(format_duration_secs(0), "0s");
        assert_eq!(format_duration_secs(45), "45s");
        assert_eq!(format_duration_secs(65), "1m 5s");
        assert_eq!(format_duration_secs(600), "10m 0s");
    }

    #[test]
    fn bytes_per_second_handles_zero_elapsed_and_normal() {
        assert_eq!(bytes_per_second(1000, Duration::ZERO), 0);
        assert_eq!(bytes_per_second(1000, Duration::from_secs(2)), 500);
        assert_eq!(bytes_per_second(0, Duration::from_secs(5)), 0);
    }

    #[test]
    fn eta_secs_uses_ceiling_remaining_over_rate() {
        assert_eq!(eta_secs(100, 1000, 100), Some(9));
        assert_eq!(eta_secs(99, 1000, 100), Some(10));
        assert_eq!(eta_secs(1000, 1000, 100), Some(0));
        assert_eq!(eta_secs(100, 1000, 0), None);
    }

    #[test]
    fn human_rate_formats_bytes_and_binary_units() {
        assert_eq!(human_rate(999), "999 B/s");
        assert_eq!(human_rate(2048), "2.0 KiB/s");
        assert_eq!(human_rate(1_572_864), "1.5 MiB/s");
    }

    #[test]
    fn unknown_size_progress_omits_percent_and_eta() {
        let progress = format_download_progress(4096, None, 2048);

        assert!(progress.contains("4.0 KiB"));
        assert!(progress.contains("(total unknown)"));
        assert!(progress.contains("2.0 KiB/s"));
        assert!(!progress.contains('%'));
        assert!(!progress.contains("eta"));
    }
}
