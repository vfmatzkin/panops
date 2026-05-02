//! End-to-end smoke test for issue #34 — proves the engine binary can find
//! its sister dylibs (`libonnxruntime.*.dylib`, `libsherpa-onnx-c-api.dylib`)
//! at runtime via the `LC_RPATH` entries set by `build.rs`.
//!
//! Without the rpath fix, copying the binary anywhere outside `target/debug`
//! fails at startup with `Library not loaded: @rpath/libonnxruntime.*`.
//! This test reproduces that failure surface (binary in a fresh directory,
//! no `DYLD_LIBRARY_PATH`) and asserts success.
//!
//! macOS-only: the rpath build.rs is gated on `cfg(target_os = "macos")`
//! and Linux/Windows have different shared-library resolution rules.

#![cfg(target_os = "macos")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn engine_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_panops-engine"))
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("tests/fixtures/audio").is_dir())
        .expect("workspace tests/fixtures/audio not found; run from a panops checkout")
        .join("tests/fixtures")
}

/// Copies the engine binary and its sister dylibs into a fresh temp dir,
/// then runs the binary from there with `PANOPS_FAKE_ASR=1` so we don't
/// need a real Whisper model. Confirms dylibs load via the rpath fix.
#[test]
fn engine_loads_sister_dylibs_via_rpath() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin = engine_bin();
    let bin_dir = bin.parent().expect("binary has parent dir");

    // Copy the binary itself.
    let dst_bin = tmp.path().join("panops-engine");
    std::fs::copy(&bin, &dst_bin).expect("copy binary");
    let mut perms = std::fs::metadata(&dst_bin)
        .expect("stat dst binary")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&dst_bin, perms).expect("chmod dst binary");

    // Copy every sherpa/onnxruntime dylib living next to the binary.
    let mut copied_dylibs = 0usize;
    for entry in std::fs::read_dir(bin_dir)
        .expect("read target dir")
        .flatten()
    {
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if name_s.starts_with("libonnxruntime") || name_s.starts_with("libsherpa-onnx") {
            let dst = tmp.path().join(&name);
            std::fs::copy(entry.path(), &dst).expect("copy dylib");
            copied_dylibs += 1;
        }
    }
    if copied_dylibs == 0 {
        // No sister dylibs in target/<profile>/ — likely a vendored or
        // system-linked ONNXRuntime build (e.g. ONNXRUNTIME_DIR set). The
        // rpath fix is still valid; this test just can't exercise it
        // because there's nothing to copy. Skip rather than fail.
        eprintln!("skipping engine_loads_sister_dylibs_via_rpath: no sister dylibs in {bin_dir:?}");
        return;
    }

    // Run from the temp dir with no DYLD_* hints — only the rpath should
    // make this work.
    let audio = fixtures_dir().join("audio").join("en_30s.wav");
    let out = Command::new(&dst_bin)
        .arg(&audio)
        .arg("--no-diarize")
        .env("PANOPS_FAKE_ASR", "1")
        // Strip any inherited DYLD_LIBRARY_PATH so we exercise pure rpath
        // resolution. Empty string is treated by dyld as unset.
        .env("DYLD_LIBRARY_PATH", "")
        .env("DYLD_FALLBACK_LIBRARY_PATH", "")
        .output()
        .expect("run engine from tempdir");

    assert!(
        out.status.success(),
        "engine failed to start from {:?}\nstatus: {}\nstderr: {}",
        tmp.path(),
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );

    // Stdout must still be the bit-clean JSON contract.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout is not valid JSON; binary started but produced garbage");
    assert_eq!(v["schema_version"], 2, "unexpected schema_version");
}
