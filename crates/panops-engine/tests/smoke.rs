use std::path::PathBuf;

#[test]
fn binary_emits_valid_json_for_en_fixture() {
    if std::env::var_os("PANOPS_MODEL").is_none() {
        eprintln!(
            "skipping binary_emits_valid_json_for_en_fixture: set PANOPS_MODEL to a local Whisper model path"
        );
        return;
    }
    let bin = env!("CARGO_BIN_EXE_panops-engine");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf();
    let audio = repo_root.join("tests/fixtures/audio/en_30s.wav");

    let mut cmd = std::process::Command::new(bin);
    cmd.arg(&audio);
    let output = cmd.output().expect("spawn panops-engine");
    assert!(
        output.status.success(),
        "binary failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    assert_eq!(v["schema_version"], 1);
    assert!(
        v["segments"].as_array().is_some_and(|a| !a.is_empty()),
        "no segments"
    );
}
