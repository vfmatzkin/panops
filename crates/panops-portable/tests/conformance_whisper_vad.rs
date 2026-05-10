//! `WhisperVad` must pass the `Vad` conformance suite. Loads the
//! bundled Silero VAD model on first run; subsequent runs use the
//! cached file. Skipped if `PANOPS_SKIP_HEAVY=1` to keep CI fast on
//! crates that don't need to exercise the real model.

use panops_core::conformance::vad::run_suite;
use panops_portable::model::{default_vad_model_path, ensure_vad_model};
use panops_portable::whisper_vad::WhisperVad;

#[test]
fn whisper_vad_passes_conformance() {
    if std::env::var("PANOPS_SKIP_HEAVY").as_deref() == Ok("1") {
        eprintln!("skipping whisper_vad conformance (PANOPS_SKIP_HEAVY=1)");
        return;
    }
    let model_path = default_vad_model_path().expect("resolve vad model path");
    let model_path = ensure_vad_model(&model_path).expect("download vad model");
    let vad = WhisperVad::new(&model_path).expect("construct WhisperVad");
    run_suite(&vad);
}
