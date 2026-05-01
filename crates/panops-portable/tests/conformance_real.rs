use std::path::{Path, PathBuf};

use panops_core::asr::AsrProvider;
use panops_core::conformance::asr::run_suite;
use panops_portable::WhisperRsAsr;
use panops_portable::model::{DEFAULT_MODEL, default_model_path, ensure_model};

fn resolve_model() -> PathBuf {
    let dest = default_model_path().expect("default model path");
    ensure_model(DEFAULT_MODEL, &dest).expect("ensure_model")
}

#[test]
fn real_adapter_is_not_fake() {
    let model = resolve_model();
    let asr = WhisperRsAsr::new(model).expect("init");
    assert!(!asr.is_fake(), "real adapter must not opt out of WER");
}

#[test]
fn real_adapter_passes_conformance() {
    let model = resolve_model();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .join("tests/fixtures");
    let asr = WhisperRsAsr::new(model).expect("init");
    run_suite(&asr, &fixtures);
}
