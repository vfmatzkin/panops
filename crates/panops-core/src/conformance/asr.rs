use std::path::Path;

use hound::WavReader;

use crate::asr::AsrProvider;
use crate::wer::wer;

/// Conformance fixture metadata. Kept as a single source of truth so the
/// fixture set, expected languages, and WER policy stay in sync.
///
/// `wer_max = None` means no WER assertion runs for this fixture, by spec:
/// see slice 02 design (`mixed_60s`: auto-detect transcript too unstable to
/// gate on) and slice 03 design (`multi_speaker_60s`: multi-voice TTS pushes
/// WER too high to gate). Single-voice fixtures keep a tight cap.
struct FixtureMeta {
    name: &'static str,
    expected_languages: &'static [&'static str],
    wer_max: Option<f32>,
}

const FIXTURES: &[FixtureMeta] = &[
    FixtureMeta {
        name: "en_30s",
        expected_languages: &["en"],
        wer_max: Some(0.20),
    },
    FixtureMeta {
        name: "es_30s",
        expected_languages: &["es"],
        wer_max: Some(0.20),
    },
    FixtureMeta {
        name: "mixed_60s",
        expected_languages: &["en", "es"],
        wer_max: None,
    },
    FixtureMeta {
        name: "multi_speaker_60s",
        expected_languages: &["en"],
        wer_max: None,
    },
];

pub fn run_suite<P: AsrProvider>(provider: &P, fixtures_dir: &Path) {
    for meta in FIXTURES {
        run_one(provider, fixtures_dir, meta);
    }
}

/// Run the ASR conformance checks against a single audio fixture.
/// Used by `panops-core` integration tests where each fixture gets
/// its own provider instance (e.g. fixture-aware `TranscriptFileFake`).
///
/// `wer_max = None` skips the WER assertion entirely. Set to `None`
/// for fakes since they echo the sidecar back (WER ≈ 0, not meaningful).
pub fn run_one_fixture<P: AsrProvider>(
    provider: &P,
    audio: &Path,
    transcript: &Path,
    expected_languages: &[&str],
    wer_max: Option<f32>,
) {
    let name = audio
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();

    let (samples, sample_rate) =
        load_wav_mono16k_inline(audio).unwrap_or_else(|e| panic!("[{name}] load wav: {e}"));

    let result = provider
        .transcribe(&samples, sample_rate, None)
        .unwrap_or_else(|e| panic!("[{name}] transcribe failed: {e}"));

    assert!(!result.segments.is_empty(), "[{name}] no segments");
    let total_audio_ms = result.audio_duration_ms;
    let mut prev_end = 0_u64;
    for (i, seg) in result.segments.iter().enumerate() {
        assert!(seg.start_ms <= seg.end_ms, "[{name}] seg[{i}] start>end");
        assert!(
            seg.end_ms <= total_audio_ms + 100,
            "[{name}] seg[{i}] end {} > audio {} + 100",
            seg.end_ms,
            total_audio_ms
        );
        assert!(
            seg.start_ms >= prev_end,
            "[{name}] seg[{i}] overlaps prev (start {} < prev_end {})",
            seg.start_ms,
            prev_end
        );
        prev_end = seg.end_ms;
    }

    let langs: Vec<&str> = result
        .segments
        .iter()
        .filter_map(|s| s.language_detected.as_deref())
        .collect();
    assert!(!langs.is_empty(), "[{name}] no language_detected populated");

    let any_match = langs.iter().any(|l| expected_languages.contains(l));
    assert!(
        any_match,
        "[{name}] expected one of {expected:?}, got {langs:?}",
        expected = expected_languages
    );

    if let Some(wer_max) = wer_max {
        let ground_truth = std::fs::read_to_string(transcript)
            .unwrap_or_else(|e| panic!("[{name}] read transcript: {e}"));
        let hypothesis = result
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let wer_value = wer(&ground_truth, &hypothesis);
        assert!(
            wer_value <= wer_max,
            "[{name}] WER {wer_value:.3} > {wer_max}\n  gt: {ground_truth:?}\n  hy: {hypothesis:?}"
        );
    }
}

fn run_one<P: AsrProvider>(provider: &P, fixtures_dir: &Path, meta: &FixtureMeta) {
    let audio = fixtures_dir
        .join("audio")
        .join(format!("{name}.wav", name = meta.name));
    let transcript_path = fixtures_dir
        .join("audio")
        .join(format!("{name}.transcript.txt", name = meta.name));

    run_one_fixture(
        provider,
        &audio,
        &transcript_path,
        meta.expected_languages,
        meta.wer_max,
    );
}

/// Inline 16 kHz mono WAV loader for the conformance harness. Mirrors
/// `panops_portable::audio::load_wav_mono16k` but lives here so
/// `panops-core` doesn't gain a `panops-portable` dep.
fn load_wav_mono16k_inline(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let reader = WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 {
        return Err(format!("expected 16 kHz, got {} Hz", spec.sample_rate));
    }
    let samples_i16: Vec<i16> = reader
        .into_samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut audio_f32 = vec![0.0_f32; samples_i16.len()];
    for (dst, src) in audio_f32.iter_mut().zip(samples_i16.iter()) {
        *dst = (*src as f32) / (i16::MAX as f32);
    }
    let audio = if spec.channels == 2 {
        audio_f32
            .chunks_exact(2)
            .map(|c| (c[0] + c[1]) / 2.0)
            .collect()
    } else if spec.channels == 1 {
        audio_f32
    } else {
        return Err(format!("expected 1 or 2 channels, got {}", spec.channels));
    };
    Ok((audio, 16_000))
}
