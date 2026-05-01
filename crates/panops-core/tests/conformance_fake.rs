use std::path::Path;

use panops_core::conformance::asr::run_suite;
use panops_core::conformance::fakes::TranscriptFileFake;

#[test]
fn fake_passes_conformance() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-core")
        .join("tests/fixtures");
    let fake = TranscriptFileFake;
    run_suite(&fake, &fixtures);
}
