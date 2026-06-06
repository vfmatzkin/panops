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
