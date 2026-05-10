use panops_core::vad::SpeechRegion;
use panops_portable::audio::{load_wav_mono16k, merge_adjacent_regions};

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("tests/fixtures/audio").is_dir())
        .unwrap()
        .join("tests/fixtures/audio")
}

#[test]
fn load_wav_mono16k_accepts_repo_fixture() {
    let path = fixtures_dir().join("en_30s.wav");
    let (samples, sr) = load_wav_mono16k(&path).expect("load en_30s.wav");
    assert_eq!(sr, 16_000);
    // 30s fixture is actually ~26.9s (431101 samples at 16 kHz).
    // Assert within ±5s of expected 480k to avoid brittle exact counts.
    let expected = 30 * 16_000;
    let diff = if samples.len() > expected {
        samples.len() - expected
    } else {
        expected - samples.len()
    };
    assert!(
        diff < 80_000,
        "expected ~480000 samples (±5s), got {}",
        samples.len()
    );
}

#[test]
fn load_wav_mono16k_rejects_nonexistent() {
    let err = load_wav_mono16k(std::path::Path::new("/nonexistent/path.wav"))
        .expect_err("should fail on missing file");
    let s = format!("{err}");
    assert!(s.contains("not found") || s.contains("io"), "got: {s}");
}

#[test]
fn load_wav_mono16k_rejects_wrong_sample_format() {
    // Write a minimal WAV header with 32-bit float samples.
    let dir = std::env::temp_dir();
    let path = dir.join("panops_test_float32.wav");
    write_wav_header(&path, 16_000, 1, 32, hound::SampleFormat::Float);
    let err = load_wav_mono16k(&path).expect_err("should reject float32");
    let s = format!("{err}");
    assert!(s.contains("sample format"), "got: {s}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_wav_mono16k_rejects_wrong_bit_depth() {
    let dir = std::env::temp_dir();
    let path = dir.join("panops_test_24bit.wav");
    write_wav_header(&path, 16_000, 1, 24, hound::SampleFormat::Int);
    let err = load_wav_mono16k(&path).expect_err("should reject 24-bit");
    let s = format!("{err}");
    assert!(s.contains("bits per sample"), "got: {s}");
    let _ = std::fs::remove_file(&path);
}

fn write_wav_header(
    path: &std::path::Path,
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    sample_format: hound::SampleFormat,
) {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample,
        sample_format,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("create temp wav");
    // Write a few samples so the file is valid enough for hound to open.
    for _ in 0..100 {
        if sample_format == hound::SampleFormat::Int {
            let _ = writer.write_sample(0i16);
        } else {
            let _ = writer.write_sample(0.0f32);
        }
    }
    writer.finalize().expect("finalize temp wav");
}

#[test]
fn merge_empty_input_returns_empty() {
    let out = merge_adjacent_regions(vec![], 5_000);
    assert!(out.is_empty());
}

#[test]
fn merge_single_region_unchanged() {
    let r = SpeechRegion {
        start_ms: 100,
        end_ms: 1_000,
    };
    let out = merge_adjacent_regions(vec![r], 5_000);
    assert_eq!(out, vec![r]);
}

#[test]
fn merge_close_adjacent_regions_combine() {
    // Gap is 1s; threshold is 5s; should merge.
    let regions = vec![
        SpeechRegion {
            start_ms: 0,
            end_ms: 2_000,
        },
        SpeechRegion {
            start_ms: 3_000,
            end_ms: 6_000,
        },
    ];
    let out = merge_adjacent_regions(regions, 5_000);
    assert_eq!(
        out,
        vec![SpeechRegion {
            start_ms: 0,
            end_ms: 6_000
        }]
    );
}

#[test]
fn merge_distant_regions_stay_separate() {
    // Gap is 9s; threshold is 5s; should NOT merge.
    let regions = vec![
        SpeechRegion {
            start_ms: 0,
            end_ms: 2_000,
        },
        SpeechRegion {
            start_ms: 11_000,
            end_ms: 15_000,
        },
    ];
    let out = merge_adjacent_regions(regions, 5_000);
    assert_eq!(out.len(), 2);
}

#[test]
fn merge_three_regions_partial_merge() {
    // (0-2000) + (3000-6000) merge (gap 1s); (15000-20000) stays.
    let regions = vec![
        SpeechRegion {
            start_ms: 0,
            end_ms: 2_000,
        },
        SpeechRegion {
            start_ms: 3_000,
            end_ms: 6_000,
        },
        SpeechRegion {
            start_ms: 15_000,
            end_ms: 20_000,
        },
    ];
    let out = merge_adjacent_regions(regions, 5_000);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].start_ms, 0);
    assert_eq!(out[0].end_ms, 6_000);
    assert_eq!(out[1].start_ms, 15_000);
    assert_eq!(out[1].end_ms, 20_000);
}

#[test]
fn merge_unsorted_input_is_sorted_first() {
    let regions = vec![
        SpeechRegion {
            start_ms: 10_000,
            end_ms: 12_000,
        },
        SpeechRegion {
            start_ms: 0,
            end_ms: 1_000,
        },
    ];
    let out = merge_adjacent_regions(regions, 5_000);
    assert_eq!(out[0].start_ms, 0);
    assert_eq!(out[1].start_ms, 10_000);
}
