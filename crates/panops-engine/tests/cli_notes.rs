//! Sanity-check the CLI parser. Real generation runs gated.

use std::process::Command;

fn engine_bin() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("target").exists())
        .unwrap()
        .join("target/debug/panops-engine")
}

#[test]
fn notes_subcommand_help_lists_expected_flags() {
    let out = Command::new(engine_bin())
        .args(["notes", "--help"])
        .output()
        .expect("run engine");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("--screenshots"), "{s}");
    assert!(s.contains("--out"), "{s}");
    assert!(s.contains("--dialect"), "{s}");
    assert!(s.contains("--no-diarize"), "{s}");
    assert!(s.contains("--llm-provider"), "{s}");
    assert!(s.contains("--language"), "{s}");
}
