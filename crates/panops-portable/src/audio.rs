//! Audio utilities for the VAD-aware ASR pipeline:
//! `load_audio_mono16k` is the entry point — it loads any CoreAudio-decodable
//! file (WAV, MOV, MP4, M4A, MP3, …) as 16 kHz mono `f32` samples, transcoding
//! non-WAV / wrong-rate input via macOS `afconvert`. `load_wav_mono16k` is the
//! direct 16 kHz-WAV reader it builds on. `ensure_wav16k` gives file-reading
//! consumers (e.g. the sherpa diarizer, which decodes the file itself) a
//! 16 kHz-WAV *path* — borrowing a ready WAV or transcoding to a temp one.
//! `merge_adjacent_regions` collapses VAD output across short gaps so
//! per-region Whisper calls get >= 30s of speech for reliable language
//! detection.

use std::path::{Path, PathBuf};

use hound::WavReader;
use panops_core::asr::AsrError;
use panops_core::vad::SpeechRegion;
use whisper_rs::{convert_integer_to_float_audio, convert_stereo_to_mono_audio};

/// Decode a WAV file at `path` into 16 kHz mono `f32` samples.
/// Accepts mono OR stereo input (downmixes stereo to mono).
/// Returns the samples and the sample rate (always 16 kHz on
/// success). Errors map to `AsrError` for now since both ASR and
/// VAD callers consume them; if a third caller appears with
/// different error needs, lift this to its own `AudioError`.
pub fn load_wav_mono16k(path: &Path) -> Result<(Vec<f32>, u32), AsrError> {
    if !path.exists() {
        return Err(AsrError::AudioNotFound(path.to_path_buf()));
    }
    let reader = WavReader::open(path).map_err(|e| AsrError::InvalidAudio(e.to_string()))?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int {
        return Err(AsrError::InvalidAudio(format!(
            "unsupported sample format {:?} (expected 16-bit PCM)",
            spec.sample_format
        )));
    }
    if spec.bits_per_sample != 16 {
        return Err(AsrError::InvalidAudio(format!(
            "unsupported bits per sample {} (expected 16)",
            spec.bits_per_sample
        )));
    }
    if spec.sample_rate != 16_000 {
        return Err(AsrError::InvalidAudio(format!(
            "expected 16 kHz, got {} Hz",
            spec.sample_rate
        )));
    }

    let samples_i16: Vec<i16> = reader
        .into_samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AsrError::InvalidAudio(e.to_string()))?;

    let mut audio_f32 = vec![0.0_f32; samples_i16.len()];
    convert_integer_to_float_audio(&samples_i16, &mut audio_f32)
        .map_err(|e| AsrError::InvalidAudio(e.to_string()))?;

    let audio = if spec.channels == 2 {
        let mono_len = audio_f32.len() / 2;
        let mut mono = vec![0.0_f32; mono_len];
        convert_stereo_to_mono_audio(&audio_f32, &mut mono)
            .map_err(|e| AsrError::InvalidAudio(e.to_string()))?;
        mono
    } else if spec.channels == 1 {
        audio_f32
    } else {
        return Err(AsrError::InvalidAudio(format!(
            "expected 1 or 2 channels, got {}",
            spec.channels
        )));
    };

    Ok((audio, 16_000))
}

fn is_wav(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("wav"))
        .unwrap_or(false)
}

/// Load audio from any container CoreAudio can decode (WAV, MOV, MP4, M4A,
/// MP3, AAC, …) as 16 kHz mono `f32` samples — the format the ASR + VAD
/// pipeline consumes. A ready 16 kHz WAV is read directly; anything else (a
/// video, a compressed audio file, or a WAV at the wrong rate/depth) is
/// transcoded to a temporary 16 kHz mono 16-bit WAV via macOS `afconvert`
/// first, then read. On non-macOS only 16 kHz WAV input is supported
/// (afconvert is macOS-only; a cross-platform decoder can be added later).
pub fn load_audio_mono16k(path: &Path) -> Result<(Vec<f32>, u32), AsrError> {
    // `ensure_wav16k` borrows a ready 16 kHz WAV or transcodes anything else
    // to a temp WAV — removed when `wav` drops, so cleanup is panic-safe even
    // if `load_wav_mono16k` unwinds on a malformed body.
    let wav = ensure_wav16k(path)?;
    load_wav_mono16k(wav.path())
}

/// A 16 kHz-WAV path for consumers that decode the audio *file* themselves
/// (e.g. the sherpa diarizer, which calls its own reader). Either borrows
/// the caller's already-16 kHz WAV, or owns a temporary transcoded copy
/// that is removed on drop. Crate-internal: the only consumers are
/// `load_audio_mono16k` (samples) and the sherpa diarizer (file path).
pub(crate) enum Wav16kPath {
    /// The caller's path was already a 16 kHz WAV; used as-is.
    Borrowed(PathBuf),
    /// A temp WAV transcoded from non-WAV / wrong-rate input; deleted on drop.
    Temp(PathBuf),
}

impl Wav16kPath {
    /// The on-disk path to a 16 kHz WAV, valid until this guard is dropped.
    pub(crate) fn path(&self) -> &Path {
        match self {
            Wav16kPath::Borrowed(p) | Wav16kPath::Temp(p) => p,
        }
    }
}

impl Drop for Wav16kPath {
    fn drop(&mut self) {
        if let Wav16kPath::Temp(p) = self {
            // Best-effort cleanup; surface failures (disk-full / read-only)
            // in logs rather than leaking temp files silently.
            if let Err(e) = std::fs::remove_file(p.as_path()) {
                tracing::warn!(error = %e, path = ?p, "failed to remove transcode temp file");
            }
        }
    }
}

/// Ensure `path` resolves to a 16 kHz WAV that file-reading audio tools can
/// decode. A ready 16 kHz WAV is borrowed unchanged; any other input (a
/// video, compressed audio, or a WAV at the wrong sample rate) is transcoded
/// to a temporary 16 kHz mono WAV via macOS `afconvert`. The returned guard
/// deletes any temp file when dropped. Mirrors [`load_audio_mono16k`] for the
/// path-consuming case (samples vs file). On non-macOS only 16 kHz WAV input
/// is supported (afconvert is macOS-only).
pub(crate) fn ensure_wav16k(path: &Path) -> Result<Wav16kPath, AsrError> {
    if !path.exists() {
        return Err(AsrError::AudioNotFound(path.to_path_buf()));
    }
    // A WAV that `load_wav_mono16k` would accept as-is (16 kHz, 16-bit PCM,
    // mono or stereo) is borrowed unchanged — no transcode. Anything else
    // (video, compressed audio, wrong rate/depth/channels) is transcoded.
    if is_wav(path) {
        if let Ok(reader) = WavReader::open(path) {
            let spec = reader.spec();
            if spec.sample_rate == 16_000
                && spec.bits_per_sample == 16
                && spec.sample_format == hound::SampleFormat::Int
                && (spec.channels == 1 || spec.channels == 2)
            {
                return Ok(Wav16kPath::Borrowed(path.to_path_buf()));
            }
        }
    }
    Ok(Wav16kPath::Temp(transcode_to_wav16k(path)?))
}

/// Transcode any CoreAudio-decodable file to a temporary 16 kHz mono 16-bit
/// WAV via macOS `afconvert`, returning the temp path. The caller owns the
/// temp file. The returned (wire-facing) error stays opaque — no path/stderr —
/// so it can't leak over the IPC boundary; the CoreAudio failure reason is
/// logged instead.
#[cfg(target_os = "macos")]
fn transcode_to_wav16k(path: &Path) -> Result<PathBuf, AsrError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "panops-transcode-{}-{}.wav",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let output = std::process::Command::new("/usr/bin/afconvert")
        .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
        .arg(path)
        .arg(&tmp)
        .output();
    match output {
        Ok(o) if o.status.success() => Ok(tmp),
        Ok(o) => {
            tracing::error!(
                path = ?path,
                exit = ?o.status.code(),
                stderr = %String::from_utf8_lossy(&o.stderr).trim(),
                "afconvert failed to decode audio"
            );
            let _ = std::fs::remove_file(&tmp);
            Err(AsrError::InvalidAudio(
                "could not decode audio (unsupported or corrupt media file)".to_string(),
            ))
        }
        Err(e) => {
            tracing::error!(error = %e, "afconvert spawn failed");
            Err(AsrError::InvalidAudio(
                "audio decoder (afconvert) unavailable".to_string(),
            ))
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn transcode_to_wav16k(path: &Path) -> Result<PathBuf, AsrError> {
    tracing::error!(path = ?path, "non-WAV audio on a platform without afconvert");
    Err(AsrError::InvalidAudio(
        "unsupported audio format: only 16 kHz WAV is supported on this platform".to_string(),
    ))
}

/// Collapse adjacent speech regions whose gap is `<= gap_ms` into
/// single contiguous regions. Pure function. Accepts unsorted input
/// (sorts internally). The 5s default the pipeline uses matches
/// whisperX's well-tested value: short pauses inside speech merge
/// (so Whisper gets >= 30s of speech for reliable language detect),
/// long pauses (turn / topic / language change) stay distinct.
pub fn merge_adjacent_regions(mut regions: Vec<SpeechRegion>, gap_ms: u64) -> Vec<SpeechRegion> {
    if regions.is_empty() {
        return regions;
    }
    regions.sort_by_key(|r| r.start_ms);
    let mut out: Vec<SpeechRegion> = Vec::with_capacity(regions.len());
    for r in regions {
        if let Some(last) = out.last_mut() {
            if r.start_ms.saturating_sub(last.end_ms) <= gap_ms {
                last.end_ms = last.end_ms.max(r.end_ms);
                continue;
            }
        }
        out.push(r);
    }
    out
}

#[cfg(all(test, target_os = "macos"))]
mod media_tests {
    use super::*;

    /// A 48 kHz WAV is rejected by `load_wav_mono16k` but `load_audio_mono16k`
    /// transcodes it (via afconvert) to 16 kHz and loads it.
    #[test]
    fn load_audio_mono16k_transcodes_non_16k_wav() {
        let dir = std::env::temp_dir().join(format!("panops-audiotest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src48k.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&src, spec).unwrap();
        for i in 0..48_000i32 {
            // 1s of a quiet tone
            w.write_sample((((i as f32) * 0.05).sin() * 6000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();

        // Direct 16k loader rejects the 48 kHz rate...
        assert!(matches!(
            load_wav_mono16k(&src),
            Err(AsrError::InvalidAudio(_))
        ));
        // ...but the media loader transcodes to 16 kHz.
        let (samples, rate) = load_audio_mono16k(&src).expect("transcode + load");
        assert_eq!(rate, 16_000);
        // 1s @ 16 kHz ≈ 16000 samples (resampled; allow tolerance).
        assert!(
            samples.len() > 12_000 && samples.len() < 20_000,
            "unexpected sample count {}",
            samples.len()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `ensure_wav16k` borrows a ready 16 kHz WAV (path unchanged, no temp)
    /// but transcodes a 48 kHz WAV to a temp 16 kHz WAV that loads cleanly —
    /// the file-path counterpart of `load_audio_mono16k`, used by the diarizer.
    #[test]
    fn ensure_wav16k_borrows_16k_and_transcodes_others() {
        let dir = std::env::temp_dir().join(format!("panops-ensurewav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let write_tone = |name: &str, rate: u32| {
            let p = dir.join(name);
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut w = hound::WavWriter::create(&p, spec).unwrap();
            for i in 0..rate as i32 {
                w.write_sample((((i as f32) * 0.05).sin() * 6000.0) as i16)
                    .unwrap();
            }
            w.finalize().unwrap();
            p
        };

        // 16 kHz WAV → borrowed as-is (no transcode, original path).
        let ready = write_tone("ready16k.wav", 16_000);
        let borrowed = ensure_wav16k(&ready).expect("borrow 16k wav");
        assert!(matches!(borrowed, Wav16kPath::Borrowed(_)));
        assert_eq!(borrowed.path(), ready.as_path());

        // 48 kHz WAV → transcoded to a temp 16 kHz WAV (different path),
        // which loads at 16 kHz.
        let high = write_tone("src48k.wav", 48_000);
        let transcoded = ensure_wav16k(&high).expect("transcode to 16k wav");
        assert!(matches!(transcoded, Wav16kPath::Temp(_)));
        assert_ne!(transcoded.path(), high.as_path());
        let (_samples, rate) = load_wav_mono16k(transcoded.path()).expect("load temp wav");
        assert_eq!(rate, 16_000);

        // Temp file is cleaned up when the guard drops.
        let temp_path = transcoded.path().to_path_buf();
        drop(transcoded);
        assert!(!temp_path.exists(), "temp wav should be removed on drop");

        std::fs::remove_dir_all(&dir).ok();
    }
}
