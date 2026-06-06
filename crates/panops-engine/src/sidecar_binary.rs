//! Shared sidecar binary validation for macOS adapter resolvers.

#[cfg(target_os = "macos")]
pub(crate) fn executable_file(path: std::path::PathBuf) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    // Canonicalize before the metadata check to narrow the symlink-swap
    // window between validation and `Command::spawn`. This is best-effort
    // UX (clean fallback on bad dev input), not a security boundary.
    let path = std::fs::canonicalize(path).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    // Verify execute permission for owner/group/other (0o111). A regular
    // file without exec bits would `spawn`-fail at runtime; cleaner to
    // reject up front and fall back to the portable provider.
    if meta.permissions().mode() & 0o111 == 0 {
        return None;
    }
    Some(path)
}

/// Locate `name` as a sibling of `dir` (the engine binary's directory in a
/// packaged bundle: `Panops.app/Contents/Resources/`). Validated via
/// `executable_file`. `dir`-parameterized so it's unit-testable.
#[cfg(target_os = "macos")]
pub(crate) fn sibling_in_dir(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    executable_file(dir.join(name))
}

/// Production resolution: `name` next to the running engine binary.
/// `None` in dev (cargo target dir has no sidecars) → caller falls back.
#[cfg(target_os = "macos")]
pub(crate) fn sibling_of_engine(name: &str) -> Option<std::path::PathBuf> {
    let exe = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    sibling_in_dir(exe.parent()?, name)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn sibling_in_dir_finds_executable_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let sc = dir.path().join("panops-asr-mac");
        std::fs::write(&sc, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&sc, std::fs::Permissions::from_mode(0o755)).unwrap();
        let got = sibling_in_dir(dir.path(), "panops-asr-mac").unwrap();
        assert_eq!(got, std::fs::canonicalize(&sc).unwrap());
    }

    #[test]
    fn sibling_in_dir_rejects_missing_or_nonexec() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sibling_in_dir(dir.path(), "nope").is_none());
        let plain = dir.path().join("panops-llm-mac");
        std::fs::write(&plain, b"x").unwrap(); // no exec bit
        assert!(sibling_in_dir(dir.path(), "panops-llm-mac").is_none());
    }
}
