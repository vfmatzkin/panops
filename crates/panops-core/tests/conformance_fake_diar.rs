use std::path::Path;

use panops_core::conformance::diar::run_suite;
use panops_core::conformance::fakes::KnownTurnsFake;

#[test]
fn fake_diarizer_passes_conformance() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-core")
        .join("tests/fixtures");
    let fake = KnownTurnsFake;
    run_suite(&fake, &fixtures);
}
