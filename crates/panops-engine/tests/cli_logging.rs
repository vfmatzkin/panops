//! Verifies the `tracing` subscriber initialized in `main()`:
//! - emits structured logs to stderr at the chosen level
//! - does NOT pollute stdout (default-mode JSON contract must remain bit-clean)
//!
//! Uses `PANOPS_FAKE_ASR=1` so we don't need a real Whisper model.

use std::process::Command;

fn engine_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_panops-engine"))
}

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("tests/fixtures/audio").is_dir())
        .unwrap()
        .join("tests/fixtures")
}

#[test]
fn default_mode_stdout_is_pure_json_with_rust_log_set() {
    // RUST_LOG=debug must NOT taint the stdout JSON contract.
    let audio = fixtures_dir().join("audio").join("en_30s.wav");
    let out = Command::new(engine_bin())
        .arg(&audio)
        .arg("--no-diarize")
        .env("PANOPS_FAKE_ASR", "1")
        .env("RUST_LOG", "debug")
        .output()
        .expect("run engine");

    assert!(
        out.status.success(),
        "engine exited with {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );

    // Stdout must be bit-clean valid JSON.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let _: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout is not valid JSON; tracing leaked into it");
}

#[test]
fn rust_log_off_silences_info_and_debug() {
    // RUST_LOG=off should suppress info/debug/warn on the success path.
    // (Error-level suppression and the `eprintln!` error path are not asserted
    // here — those go through plain stderr writes, not tracing.)
    let audio = fixtures_dir().join("audio").join("en_30s.wav");
    let out = Command::new(engine_bin())
        .arg(&audio)
        .arg("--no-diarize")
        .env("PANOPS_FAKE_ASR", "1")
        .env("RUST_LOG", "off")
        .output()
        .expect("run engine");
    assert!(out.status.success(), "engine should succeed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("INFO") && !stderr.contains("DEBUG") && !stderr.contains("WARN"),
        "RUST_LOG=off should suppress info/debug/warn; got stderr:\n{stderr}"
    );
}
