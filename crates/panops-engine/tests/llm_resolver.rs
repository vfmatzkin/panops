#![cfg(target_os = "macos")]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write_fake_sidecar(
    available: bool,
) -> Result<(tempfile::TempDir, PathBuf), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("fake-panops-llm-mac");
    let result = if available {
        r#"{"available":true}"#
    } else {
        r#"{"available":false,"reason":"fake unavailable"}"#
    };
    let script = format!(
        r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')
  printf '{{"jsonrpc":"2.0","id":%s,"result":{result}}}\n' "$id"
done
"#,
        result = result
    );
    std::fs::write(&path, script)?;
    let mut perms = std::fs::metadata(&path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms)?;
    Ok((dir, path))
}

#[test]
fn resolver_accepts_available_sidecar() -> TestResult {
    let (_dir, bin) = write_fake_sidecar(true)?;
    let rt = tokio::runtime::Runtime::new()?;
    let (llm, info) = panops_engine::llm_resolver::pick_llm(rt.handle().clone(), Some(bin));
    assert_eq!(info.provider, "apple-foundation");
    assert_eq!(info.model, "on-device");
    assert!(info.local);
    drop(llm);
    Ok(())
}

#[test]
fn resolver_falls_back_when_probe_unavailable() -> TestResult {
    let (_dir, bin) = write_fake_sidecar(false)?;
    let rt = tokio::runtime::Runtime::new()?;
    let (llm, info) = panops_engine::llm_resolver::pick_llm(rt.handle().clone(), Some(bin));
    assert_eq!(info.provider, "ollama");
    assert_eq!(info.model, "gemma3:4b");
    assert!(info.local);
    drop(llm);
    Ok(())
}

#[test]
fn resolver_falls_back_when_sidecar_is_not_executable() -> TestResult {
    let dir = tempfile::tempdir()?;
    let bin = dir.path().join("not-executable");
    std::fs::write(&bin, "#!/bin/sh\nexit 0\n")?;
    let rt = tokio::runtime::Runtime::new()?;
    let (llm, info) = panops_engine::llm_resolver::pick_llm(rt.handle().clone(), Some(bin));
    assert_eq!(info.provider, "ollama");
    assert_eq!(info.model, "gemma3:4b");
    assert!(info.local);
    drop(llm);
    Ok(())
}

#[test]
fn resolver_falls_back_when_sidecar_path_does_not_exist() -> TestResult {
    let dir = tempfile::tempdir()?;
    let missing = dir.path().join("missing-panops-llm-mac");
    let rt = tokio::runtime::Runtime::new()?;
    let (llm, info) = panops_engine::llm_resolver::pick_llm(rt.handle().clone(), Some(missing));
    assert_eq!(info.provider, "ollama");
    assert_eq!(info.model, "gemma3:4b");
    assert!(info.local);
    drop(llm);
    Ok(())
}
