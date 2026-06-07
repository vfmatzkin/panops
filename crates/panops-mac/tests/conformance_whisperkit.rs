//! Conformance harness for `WhisperKitAsr`. Self-skips when
//! `PANOPS_ASR_SIDECAR_BIN` is unset OR `PANOPS_SKIP_HEAVY=1`.
//! Runs in the CI heavy-test job and locally when the built sidecar binary
//! is available.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};

use panops_core::asr::AsrProvider;
use panops_core::conformance::asr::run_suite;
use panops_mac::WhisperKitAsr;

fn sidecar_bin() -> Option<PathBuf> {
    std::env::var_os("PANOPS_ASR_SIDECAR_BIN").map(PathBuf::from)
}

fn heavy_skipped() -> bool {
    std::env::var("PANOPS_SKIP_HEAVY").as_deref() == Ok("1")
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-mac")
        .join("tests/fixtures")
}

#[test]
fn whisperkit_is_not_fake() {
    if heavy_skipped() {
        return;
    }
    let Some(bin) = sidecar_bin() else {
        return;
    };
    let asr = WhisperKitAsr::new(bin);
    assert!(!asr.is_fake(), "real adapter must not opt out of WER");
}

#[test]
fn whisperkit_passes_full_conformance() {
    if heavy_skipped() {
        eprintln!("skipping whisperkit_passes_full_conformance (PANOPS_SKIP_HEAVY=1)");
        return;
    }
    let Some(bin) = sidecar_bin() else {
        eprintln!(
            "skipping whisperkit_passes_full_conformance: \
             set PANOPS_ASR_SIDECAR_BIN to the built panops-asr-mac binary"
        );
        return;
    };
    let asr = WhisperKitAsr::new(bin);
    let fixtures = fixtures_dir();

    // Full suite: en_30s, es_30s, mixed_60s, multi_speaker_60s.
    // Fix #125: explicitly enable detectLanguage when no hint provided,
    // improving Spanish auto-detection on tiny/base models.
    run_suite(&asr, &fixtures);
}
