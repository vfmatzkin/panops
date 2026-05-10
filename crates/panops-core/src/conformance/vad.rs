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
//! The harness generates its own synthetic audio (silence + a sine
//! wave burst) so it doesn't depend on fixture files.

use crate::vad::{Vad, VadError};

const SR: u32 = 16_000;

/// Run the full conformance suite against a `Vad` implementation.
pub fn run_suite<V: Vad>(adapter: &V) {
    detects_speech_in_simple_burst(adapter);
    returns_sorted_non_overlapping_regions(adapter);
    rejects_non_16khz_sample_rate(adapter);
    handles_silence_without_panic(adapter);
}

fn detects_speech_in_simple_burst<V: Vad>(adapter: &V) {
    // 5s total: 1s silence + 2s tone + 2s silence.
    let mut samples = vec![0.0_f32; SR as usize]; // 1s silence
    samples.extend(sine_wave(2 * SR as usize, 440.0)); // 2s tone
    samples.extend(vec![0.0_f32; 2 * SR as usize]); // 2s silence

    let regions = adapter
        .detect_speech(&samples, SR)
        .expect("detect_speech on simple burst should succeed");
    assert!(
        !regions.is_empty(),
        "expected >=1 speech region for a 2s tone; got 0"
    );
    let total_ms = (samples.len() as u64 * 1000) / u64::from(SR);
    for (i, r) in regions.iter().enumerate() {
        assert!(r.start_ms < r.end_ms, "region[{i}] start>=end: {r:?}");
        assert!(
            r.end_ms <= total_ms + 100,
            "region[{i}] end {} > audio {} + 100",
            r.end_ms,
            total_ms
        );
    }
}

fn returns_sorted_non_overlapping_regions<V: Vad>(adapter: &V) {
    // Two tones separated by silence: 0.5s tone + 0.5s silence + 0.5s tone.
    let mut samples = sine_wave((SR / 2) as usize, 440.0);
    samples.extend(vec![0.0_f32; (SR / 2) as usize]);
    samples.extend(sine_wave((SR / 2) as usize, 440.0));

    let regions = adapter.detect_speech(&samples, SR).unwrap();
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

fn sine_wave(n_samples: usize, freq_hz: f32) -> Vec<f32> {
    use std::f32::consts::TAU;
    let mut out = Vec::with_capacity(n_samples);
    for i in 0..n_samples {
        let t = i as f32 / SR as f32;
        out.push(0.5 * (TAU * freq_hz * t).sin());
    }
    out
}
