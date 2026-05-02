//! Integration test for the default-mode CLI (`panops-engine <wav>`).
//!
//! Exercises the full default-mode path (arg parsing → fake ASR → JSON
//! stdout) without downloading or loading a real Whisper model.
//! Uses `PANOPS_FAKE_ASR=1` to swap in `TranscriptFileFake` and the
//! `en_30s.wav` fixture (which has a `.transcript.txt` sidecar).

use std::process::Command;

fn engine_bin() -> std::path::PathBuf {
    // Cargo sets CARGO_BIN_EXE_<name> for integration tests in the same crate.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_panops-engine"))
}

fn fixtures_dir() -> std::path::PathBuf {
    // tests/fixtures lives at the workspace root (two levels above this crate).
    // Detect it by the presence of the audio subdirectory to avoid matching the
    // crate's own `tests/` directory first.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("tests/fixtures/audio").is_dir())
        .unwrap()
        .join("tests/fixtures")
}

#[test]
fn default_mode_outputs_valid_json_transcript() {
    let audio = fixtures_dir().join("audio").join("en_30s.wav");
    assert!(audio.exists(), "fixture not found: {audio:?}");

    let out = Command::new(engine_bin())
        .arg(&audio)
        .arg("--no-diarize")
        .env("PANOPS_FAKE_ASR", "1")
        .output()
        .expect("failed to run panops-engine");

    assert!(
        out.status.success(),
        "engine exited with {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is not valid JSON");

    assert_eq!(json["schema_version"], 2, "unexpected schema_version");
    let segments = json["segments"]
        .as_array()
        .expect("segments missing or not array");
    assert!(!segments.is_empty(), "transcript has no segments");
    assert_eq!(
        json["diarized"], false,
        "diarized should be false with --no-diarize"
    );
    assert!(
        json["audio_duration_ms"].as_u64().unwrap_or(0) > 0,
        "audio_duration_ms should be positive",
    );
}

#[test]
fn default_mode_missing_audio_exits_nonzero() {
    let out = Command::new(engine_bin())
        .arg("/nonexistent/path/to/audio.wav")
        .arg("--no-diarize")
        .env("PANOPS_FAKE_ASR", "1")
        .output()
        .expect("failed to run panops-engine");

    assert!(
        !out.status.success(),
        "engine should exit non-zero for missing audio",
    );
}
