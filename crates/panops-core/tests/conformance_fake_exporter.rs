//! Validates the conformance harness against the in-tree fake exporter.
//! Real exporters (in `panops-portable`) run the same suite from their own
//! integration tests.

use panops_core::conformance::exporter::run_suite;
use panops_core::conformance::fakes::FakeNotesExporter;

#[test]
fn fake_passes_exporter_conformance() {
    run_suite(&FakeNotesExporter);
}
