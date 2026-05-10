use std::path::Path;

use panops_core::conformance::asr::run_one_fixture;
use panops_core::conformance::fakes::{TranscriptFileFake, read_canned_sidecar};

struct FixtureCase {
    name: &'static str,
    expected_languages: &'static [&'static str],
}

const FIXTURES: &[FixtureCase] = &[
    FixtureCase {
        name: "en_30s",
        expected_languages: &["en"],
    },
    FixtureCase {
        name: "es_30s",
        expected_languages: &["es"],
    },
    FixtureCase {
        name: "mixed_60s",
        expected_languages: &["en", "es"],
    },
    FixtureCase {
        name: "multi_speaker_60s",
        expected_languages: &["en"],
    },
];

#[test]
fn fake_passes_conformance_per_fixture() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root above crates/panops-core")
        .join("tests/fixtures");

    for case in FIXTURES {
        let audio_path = fixtures.join("audio").join(format!("{}.wav", case.name));
        let transcript_path = fixtures
            .join("audio")
            .join(format!("{}.transcript.txt", case.name));

        let canned = read_canned_sidecar(&audio_path);
        let fake = TranscriptFileFake::with_canned(canned);

        // Fakes echo the sidecar back; WER is meaningless.
        run_one_fixture(
            &fake,
            &audio_path,
            &transcript_path,
            case.expected_languages,
            None,
        );
    }
}
