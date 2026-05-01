use std::path::{Path, PathBuf};

use panops_core::conformance::diar::run_suite;
use panops_portable::SherpaDiarizer;

fn paths() -> Option<(PathBuf, PathBuf)> {
    let seg = std::env::var_os("PANOPS_DIAR_SEG").map(PathBuf::from)?;
    let emb = std::env::var_os("PANOPS_DIAR_EMB").map(PathBuf::from)?;
    Some((seg, emb))
}

#[test]
fn real_diarizer_passes_conformance() {
    let Some((seg, emb)) = paths() else {
        eprintln!(
            "skipping real_diarizer_passes_conformance: set PANOPS_DIAR_SEG and PANOPS_DIAR_EMB"
        );
        return;
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .join("tests/fixtures");
    let diar = SherpaDiarizer::new(seg, emb).expect("init");
    run_suite(&diar, &fixtures);
}
