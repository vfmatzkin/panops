//! Audio utilities for the VAD-aware ASR pipeline:
//! `load_audio_mono16k` is the entry point — it loads any CoreAudio-decodable
//! file (WAV, MOV, MP4, M4A, MP3, …) as 16 kHz mono `f32` samples, transcoding
//! non-WAV / wrong-rate input via macOS `afconvert`. `load_wav_mono16k` is the
//! direct 16 kHz-WAV reader it builds on. `merge_adjacent_regions` collapses
//! VAD output across short gaps so per-region Whisper calls get >= 30s of
//! speech for reliable language detection.

use std::path::Path;

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
    if !path.exists() {
        return Err(AsrError::AudioNotFound(path.to_path_buf()));
    }
    // Fast path: a ready 16 kHz WAV loads directly (no transcode).
    if is_wav(path) {
        match load_wav_mono16k(path) {
            Ok(out) => return Ok(out),
            // Wrong rate/depth/format → fall through to transcode + retry.
            Err(AsrError::InvalidAudio(_)) => {}
            Err(e) => return Err(e),
        }
    }
    transcode_to_wav16k_and_load(path)
}

#[cfg(target_os = "macos")]
fn transcode_to_wav16k_and_load(path: &Path) -> Result<(Vec<f32>, u32), AsrError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "panops-transcode-{}-{}.wav",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    // macOS CoreAudio `afconvert`: decode the input's audio track to
    // 16 kHz mono signed-16-bit little-endian WAV. Capture output so the
    // CoreAudio failure reason is logged; the returned (wire-facing) error
    // stays opaque — no path/stderr — so it can't leak over the IPC boundary.
    let output = std::process::Command::new("/usr/bin/afconvert")
        .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
        .arg(path)
        .arg(&tmp)
        .output();
    let result = match output {
        Ok(o) if o.status.success() => load_wav_mono16k(&tmp),
        Ok(o) => {
            tracing::error!(
                path = ?path,
                exit = ?o.status.code(),
                stderr = %String::from_utf8_lossy(&o.stderr).trim(),
                "afconvert failed to decode audio"
            );
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
    };
    let _ = std::fs::remove_file(&tmp);
    result
}

#[cfg(not(target_os = "macos"))]
fn transcode_to_wav16k_and_load(path: &Path) -> Result<(Vec<f32>, u32), AsrError> {
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
}
