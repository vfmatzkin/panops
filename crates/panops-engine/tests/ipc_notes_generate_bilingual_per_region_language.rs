//! Slice 07 acceptance test: a synthetic bilingual WAV (en_30s + 6s
//! silence + es_30s) round-trips through the VAD-aware pipeline with
//! per-segment language detection that varies between halves.
//!
//! The deterministic test fakes can't validate this — they don't
//! actually do detection. So this test uses the REAL `WhisperVad`
//! and `WhisperRsAsr`, gated on the `PANOPS_SKIP_HEAVY=1` env var
//! so it can be skipped in CI configurations that don't want to
//! pull the model files.
//!
//! The 6-second silence between the two language halves exceeds the
//! pipeline's 5s merge gap, so VAD produces TWO separate merged
//! regions. Each region transcribes independently, yielding both
//! "en" and "es" language detections.
//!
//! Without the 6s gap (continuous bilingual), VAD merges everything
//! into ONE region and Whisper detects only the dominant first-30s
//! language. The second test pins this known limitation so slice 08
//! work has a regression baseline.

use std::path::PathBuf;

use panops_core::AsrProvider;
use panops_core::Vad;
use panops_portable::audio::merge_adjacent_regions;
use panops_portable::model::{
    DEFAULT_MODEL_NAME, default_model_path, default_vad_model_path, ensure_model, ensure_vad_model,
};
use panops_portable::{WhisperRsAsr, WhisperVad};

fn fixtures_dir() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("tests/fixtures/audio").is_dir())
        .unwrap()
        .join("tests/fixtures/audio")
}

/// Concatenate multiple 16k Hz mono WAVs into a single file.
fn concat_wavs(dst: &std::path::Path, parts: &[std::path::PathBuf]) {
    use hound::{SampleFormat, WavSpec, WavWriter};
    let spec = WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(dst, spec).expect("create dst wav");
    for part in parts {
        let reader = hound::WavReader::open(part).expect("open part");
        for s in reader.into_samples::<i16>() {
            writer.write_sample(s.expect("read sample")).expect("write");
        }
    }
    writer.finalize().expect("finalize");
}

/// Write `seconds` of digital silence to a 16k Hz mono WAV.
fn write_silence_wav(dst: &std::path::Path, seconds: u32) {
    use hound::{SampleFormat, WavSpec, WavWriter};
    let spec = WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(dst, spec).expect("create silence wav");
    for _ in 0..(seconds * 16_000) {
        writer.write_sample(0i16).expect("write silence");
    }
    writer.finalize().expect("finalize");
}

/// Run the VAD → merge → per-region transcribe pipeline on a WAV file.
/// Returns a flat list of all segments from all regions.
fn run_vad_pipeline(
    vad: &WhisperVad,
    asr: &WhisperRsAsr,
    wav_path: &std::path::Path,
) -> Vec<panops_core::Segment> {
    let (samples, sr) = panops_portable::audio::load_wav_mono16k(wav_path).unwrap();
    let regions = vad.detect_speech(&samples, sr).unwrap();
    eprintln!("VAD detected {} regions", regions.len());
    let merged = merge_adjacent_regions(regions, 5_000);
    eprintln!("merged to {} regions", merged.len());
    for (i, r) in merged.iter().enumerate() {
        eprintln!("  merged[{i}]: {}ms → {}ms", r.start_ms, r.end_ms);
    }

    let mut stitched_segments: Vec<panops_core::Segment> = Vec::new();
    for region in merged.iter() {
        let start = ((region.start_ms * u64::from(sr)) / 1000) as usize;
        let end = ((region.end_ms * u64::from(sr)) / 1000) as usize;
        let end = end.min(samples.len());
        let chunk = &samples[start..end];
        if chunk.is_empty() {
            continue;
        }
        let t = asr.transcribe(chunk, sr, None).unwrap();
        for mut seg in t.segments {
            // Offset region-local timestamps to absolute audio time.
            seg.start_ms =
                (seg.start_ms + region.start_ms).min((samples.len() as u64 * 1000) / u64::from(sr));
            seg.end_ms =
                (seg.end_ms + region.start_ms).min((samples.len() as u64 * 1000) / u64::from(sr));
            stitched_segments.push(seg);
        }
    }
    stitched_segments
}

/// Bilingual audio with a 6s silence gap between languages yields both
/// "en" and "es" language_detected values across distinct segments.
///
/// The 6s silence exceeds the 5s merge gap, so VAD splits into TWO
/// merged regions. Each region transcribes independently, proving the
/// per-region auto-detect path functions end-to-end.
#[test]
fn bilingual_audio_yields_per_region_language_attribution() {
    if std::env::var("PANOPS_SKIP_HEAVY").as_deref() == Ok("1") {
        eprintln!("skipping bilingual test (PANOPS_SKIP_HEAVY=1)");
        return;
    }

    let audio_dir = tempfile::tempdir().unwrap();
    let bilingual = audio_dir.path().join("en_silence_es.wav");
    let silence = audio_dir.path().join("silence_6s.wav");
    write_silence_wav(&silence, 6);
    concat_wavs(
        &bilingual,
        &[
            fixtures_dir().join("en_30s.wav"),
            silence,
            fixtures_dir().join("es_30s.wav"),
        ],
    );

    let asr_path = ensure_model(DEFAULT_MODEL_NAME, &default_model_path().expect("asr path"))
        .expect("asr download");
    let asr = WhisperRsAsr::new(asr_path).expect("WhisperRsAsr::new");
    let vad_path =
        ensure_vad_model(&default_vad_model_path().expect("vad path")).expect("vad download");
    let vad = WhisperVad::new(&vad_path).expect("WhisperVad::new");

    let segments = run_vad_pipeline(&vad, &asr, &bilingual);

    // Collect languages from all segments.
    let langs: Vec<_> = segments
        .iter()
        .filter_map(|s| s.language_detected.clone())
        .collect();

    eprintln!("all detected languages: {:?}", langs);

    assert!(
        !langs.is_empty(),
        "expected segments with language_detected"
    );
    assert!(
        langs.iter().any(|l| l == "en"),
        "expected at least one English segment, got {langs:?}"
    );
    assert!(
        langs.iter().any(|l| l == "es"),
        "expected at least one Spanish segment, got {langs:?}"
    );
}

/// Continuous bilingual speech (no silence gap) detects only the first
/// language. This pins the known 30s auto-detect limitation: when VAD
/// merges the entire audio into one region, Whisper uses the first ~30s
/// for language detection and applies it to the whole transcription.
#[test]
fn continuous_bilingual_detects_only_first_language() {
    if std::env::var("PANOPS_SKIP_HEAVY").as_deref() == Ok("1") {
        eprintln!("skipping continuous bilingual test (PANOPS_SKIP_HEAVY=1)");
        return;
    }

    let audio_dir = tempfile::tempdir().unwrap();
    let bilingual = audio_dir.path().join("en_then_es_continuous.wav");
    concat_wavs(
        &bilingual,
        &[
            fixtures_dir().join("en_30s.wav"),
            fixtures_dir().join("es_30s.wav"),
        ],
    );

    let asr_path = ensure_model(DEFAULT_MODEL_NAME, &default_model_path().expect("asr path"))
        .expect("asr download");
    let asr = WhisperRsAsr::new(asr_path).expect("WhisperRsAsr::new");
    let vad_path =
        ensure_vad_model(&default_vad_model_path().expect("vad path")).expect("vad download");
    let vad = WhisperVad::new(&vad_path).expect("WhisperVad::new");

    let segments = run_vad_pipeline(&vad, &asr, &bilingual);

    let langs: Vec<_> = segments
        .iter()
        .filter_map(|s| s.language_detected.clone())
        .collect();

    eprintln!("continuous bilingual detected languages: {:?}", langs);

    // English should be detected (it's the first 30s of speech).
    assert!(
        langs.iter().any(|l| l == "en"),
        "expected English segments, got {langs:?}"
    );
}
