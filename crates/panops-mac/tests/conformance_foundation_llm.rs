//! Conformance harness for `FoundationLlm` using a fake sidecar binary
//! that speaks the same newline-delimited JSON-RPC protocol as
//! `panops-llm-mac`.

#![cfg(target_os = "macos")]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use panops_core::conformance::llm::run_suite;
use panops_mac::FoundationLlm;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write_fake_sidecar(
    available: bool,
) -> Result<(tempfile::TempDir, PathBuf), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("fake-panops-llm-mac");
    let probe = if available {
        r#"{"available":true}"#
    } else {
        r#"{"available":false,"reason":"fake unavailable"}"#
    };
    let script = format!(
        r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')
  if printf '%s' "$line" | grep -q '"method":"probe"'; then
    printf '{{"jsonrpc":"2.0","id":%s,"result":{probe}}}\n' "$id"
  elif printf '%s' "$line" | grep -q '"method":"complete"'; then
    if printf '%s' "$line" | grep -q '"schema"'; then
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"json":{{"ok":true}}}}}}\n' "$id"
    else
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"text":"hello from fake foundation"}}}}\n' "$id"
    fi
  else
    printf '{{"jsonrpc":"2.0","id":%s,"error":{{"code":-32601,"message":"method not found"}}}}\n' "$id"
  fi
done
"#,
        probe = probe
    );
    std::fs::write(&path, script)?;
    let mut perms = std::fs::metadata(&path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms)?;
    Ok((dir, path))
}

fn write_static_response_sidecar(
    response: &'static str,
) -> Result<(tempfile::TempDir, PathBuf), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("fake-panops-llm-mac-static");
    let script = format!(
        r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' '{response}'
done
"#,
        response = response.replace('\'', r"'\''")
    );
    std::fs::write(&path, script)?;
    let mut perms = std::fs::metadata(&path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms)?;
    Ok((dir, path))
}

#[test]
fn fake_foundation_llm_passes_conformance_suite() -> TestResult {
    let (_dir, bin) = write_fake_sidecar(true)?;
    let llm = FoundationLlm::new(bin);
    let probe = llm.probe()?;
    assert!(probe.available);
    run_suite(&llm);
    Ok(())
}

#[test]
fn fake_probe_can_report_unavailable() -> TestResult {
    let (_dir, bin) = write_fake_sidecar(false)?;
    let llm = FoundationLlm::new(bin);
    let probe = llm.probe()?;
    assert!(!probe.available);
    assert_eq!(probe.reason.as_deref(), Some("fake unavailable"));
    Ok(())
}

#[test]
fn rejects_jsonrpc_id_mismatch() -> TestResult {
    let (_dir, bin) =
        write_static_response_sidecar(r#"{"jsonrpc":"2.0","id":999,"result":{"available":true}}"#)?;
    let llm = FoundationLlm::new(bin);
    let err = llm.probe().expect_err("mismatched id must fail");
    assert!(
        matches!(err, panops_core::llm::LlmError::Provider(ref message) if message.contains("JSON-RPC id mismatch")),
        "unexpected error: {err:?}"
    );
    Ok(())
}

#[test]
fn rejects_jsonrpc_version_mismatch() -> TestResult {
    let (_dir, bin) =
        write_static_response_sidecar(r#"{"jsonrpc":"1.0","id":1,"result":{"available":true}}"#)?;
    let llm = FoundationLlm::new(bin);
    let err = llm
        .probe()
        .expect_err("unsupported JSON-RPC version must fail");
    assert!(
        matches!(err, panops_core::llm::LlmError::Provider(ref message) if message.contains("unsupported jsonrpc version")),
        "unexpected error: {err:?}"
    );
    Ok(())
}
