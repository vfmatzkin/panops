//! Conformance harness for [`crate::vad::Vad`] adapters.
//!
//! Every Vad impl (real `WhisperVad`, fake `KnownRegionsFake`) must
//! pass this same suite. Asserts the contract documented on the
//! trait:
//!
//! - `detect_speech` accepts mono 16 kHz samples without crashing.
//! - Returned regions are sorted by `start_ms`.
//! - Each region has `start_ms < end_ms`.
//! - Regions don't overlap.
//! - All region boundaries fall within the audio duration.
//! - Fully-silent input returns 0 regions (or 1 small region for
//!   adapters that have a noise floor; we accept 0..=1 here).
//! - Non-16 kHz input returns `VadError::InvalidAudio`.
//!
//! Speech detection tests use `tests/fixtures/audio/en_30s.wav` (real
//! synthesized English speech) so that Silera-based adapters (which
//! reject pure-tone sine waves) pass the harness. Silence tests still
//! use synthetic `vec![0.0; …]`.

use std::path::Path;

use crate::vad::{Vad, VadError};

const SR: u32 = 16_000;

/// Load `en_30s.wav` from the workspace fixtures directory once and
/// return a borrowed slice for reuse across tests.
fn load_speech_fixture() -> Vec<f32> {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("tests/fixtures/audio").is_dir())
        .expect("workspace root with tests/fixtures/audio not found");
    let wav_path = workspace_root.join("tests/fixtures/audio/en_30s.wav");
    let mut reader = hound::WavReader::open(&wav_path)
        .unwrap_or_else(|_| panic!("open fixture {}", wav_path.display()));
    let spec = reader.spec();
    assert!(
        spec.sample_format == hound::SampleFormat::Int && spec.bits_per_sample == 16,
        "fixture {} must be 16-bit PCM, got {:?} {}-bit",
        wav_path.display(),
        spec.sample_format,
        spec.bits_per_sample
    );
    reader
        .samples::<i16>()
        .map(|r| r.expect("decode fixture sample") as f32 / i16::MAX as f32)
        .collect()
}

/// Run the full conformance suite against a `Vad` implementation.
pub fn run_suite<V: Vad>(adapter: &V) {
    let speech_samples = load_speech_fixture();
    detects_speech_in_simple_burst(adapter, &speech_samples);
    returns_sorted_non_overlapping_regions(adapter, &speech_samples);
    rejects_non_16khz_sample_rate(adapter);
    handles_silence_without_panic(adapter);
}

fn detects_speech_in_simple_burst<V: Vad>(adapter: &V, samples: &[f32]) {
    let total_ms = (samples.len() as u64 * 1000) / u64::from(SR);

    let regions = adapter
        .detect_speech(samples, SR)
        .expect("detect_speech on real speech should succeed");
    assert!(
        !regions.is_empty(),
        "expected >=1 speech region for 30s of speech; got 0"
    );
    let mut prev_end = 0_u64;
    for (i, r) in regions.iter().enumerate() {
        assert!(
            r.start_ms >= prev_end,
            "region[{i}] not sorted (start {} < prev_end {})",
            r.start_ms,
            prev_end
        );
        prev_end = r.end_ms;
        assert!(r.start_ms < r.end_ms, "region[{i}] start>=end: {r:?}");
        assert!(
            r.end_ms <= total_ms,
            "region[{i}] end {} > audio duration {}",
            r.end_ms,
            total_ms
        );
    }
}

fn returns_sorted_non_overlapping_regions<V: Vad>(adapter: &V, full: &[f32]) {
    // Split the 30s fixture into two halves separated by 1s of silence.
    let half = full.len() / 2;
    let mut samples = full[..half].to_vec();
    samples.extend(vec![0.0_f32; SR as usize]); // 1s silence gap
    samples.extend(&full[half..]);

    let regions = adapter.detect_speech(&samples, SR).unwrap();
    assert!(
        !regions.is_empty(),
        "returns_sorted_non_overlapping_regions: expected >=1 region for real speech, got 0"
    );
    let mut prev_end = 0_u64;
    for (i, r) in regions.iter().enumerate() {
        assert!(
            r.start_ms >= prev_end,
            "region[{i}] overlaps prev (start {} < prev_end {})",
            r.start_ms,
            prev_end
        );
        prev_end = r.end_ms;
    }
}

fn rejects_non_16khz_sample_rate<V: Vad>(adapter: &V) {
    let samples = vec![0.0_f32; 8000];
    let err = adapter
        .detect_speech(&samples, 8_000)
        .expect_err("8 kHz should be rejected");
    assert!(
        matches!(err, VadError::InvalidAudio(_)),
        "expected InvalidAudio, got {err:?}"
    );
}

fn handles_silence_without_panic<V: Vad>(adapter: &V) {
    // 3s of digital silence — adapter may return 0 regions or 1
    // small region depending on its noise-floor handling. Both are
    // acceptable; we only assert no panic and no error.
    let samples = vec![0.0_f32; 3 * SR as usize];
    let regions = adapter
        .detect_speech(&samples, SR)
        .expect("silence detection must not error");
    assert!(
        regions.len() <= 1,
        "expected 0..=1 regions for silence, got {}",
        regions.len()
    );
}
