use std::path::{Path, PathBuf};

use panops_core::asr::AsrProvider;
use panops_core::conformance::asr::run_suite;
use panops_portable::WhisperRsAsr;

fn explicit_model() -> Option<PathBuf> {
    std::env::var_os("PANOPS_MODEL").map(PathBuf::from)
}

#[test]
fn real_adapter_is_not_fake() {
    let Some(model) = explicit_model() else {
        eprintln!(
            "skipping real_adapter_is_not_fake: set PANOPS_MODEL to a local Whisper model path"
        );
        return;
    };
    let asr = WhisperRsAsr::new(model).expect("init");
    assert!(!asr.is_fake(), "real adapter must not opt out of WER");
}

#[test]
fn real_adapter_passes_conformance() {
    let Some(model) = explicit_model() else {
        eprintln!(
            "skipping real_adapter_passes_conformance: set PANOPS_MODEL to a local Whisper model path"
        );
        return;
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .join("tests/fixtures");
    let asr = WhisperRsAsr::new(model).expect("init");
    run_suite(&asr, &fixtures);
}
