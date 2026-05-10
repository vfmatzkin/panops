//! Pure utilities used by the VAD-aware ASR pipeline:
//! `load_wav_mono16k` decodes a WAV file into 16 kHz mono `f32`
//! samples (the format `WhisperRsAsr` and `WhisperVad` both accept).
//! `merge_adjacent_regions` collapses VAD output across short gaps
//! so per-region Whisper calls get >= 30s of speech for reliable
//! language detection.

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
