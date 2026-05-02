//! UDS lifecycle helpers used by `run_serve`.
//!
//! The probe-then-unlink dance prevents two engine instances from racing
//! on the same socket path while still recovering from a stale file
//! left after a crash. Filesystem perms `0600` are slice 05's only
//! auth mechanism.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

/// Default socket location on macOS.
pub(super) fn default_socket_path() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    if home.is_empty() {
        return Err("HOME is empty".to_string());
    }
    let home_path = std::path::Path::new(&home);
    if !home_path.is_absolute() {
        return Err(format!("HOME is not an absolute path: {home}"));
    }
    Ok(home_path.join("Library/Application Support/panops/engine.sock"))
}

/// Run `f` with the process umask temporarily set to `0o077`, restoring
/// the previous umask afterwards.
///
/// SAFETY: `libc::umask` mutates process-global state and is racy across
/// threads. `bind_with_lifecycle` is called once at startup before any
/// concurrent server activity, so this is safe in our usage.
unsafe fn with_strict_umask<F, T>(f: F) -> T
where
    F: FnOnce() -> T,
{
    // libc::umask returns the previous umask and sets the new one.
    // SAFETY: caller upholds the no-concurrent-thread invariant documented
    // on this fn; libc::umask itself is FFI and must be wrapped in unsafe
    // (Rust 2024 unsafe_op_in_unsafe_fn).
    let prev = unsafe { libc::umask(0o077) };
    let result = f();
    unsafe { libc::umask(prev) };
    result
}

/// Bind a `UnixListener` at `path` after probing for a live engine and
/// removing any stale socket file. Sets `0600` perms on the socket.
pub(super) async fn bind_with_lifecycle(path: &Path) -> Result<UnixListener, BindError> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| BindError::Bind(format!("create parent {parent:?}: {e}")))?;
            // Tighten parent perms (best-effort).
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }

    if path.exists() {
        // Probe for a live listener with a short timeout. Local UDS
        // connect is sub-millisecond on a healthy box; 250ms covers
        // paged-out kernel state under load (e.g. fresh boot, heavy
        // disk pressure) without making a stale-socket recovery feel
        // sluggish.
        let probe = timeout(Duration::from_millis(250), UnixStream::connect(path)).await;
        match probe {
            // Live listener answered — refuse to steal it.
            Ok(Ok(_)) => return Err(BindError::EngineAlreadyRunning(path.to_path_buf())),

            // `ConnectionRefused` is the canonical "socket file exists,
            // nobody is listen()ing" — a previous engine crashed without
            // unlinking. Safe to remove.
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                std::fs::remove_file(path)
                    .map_err(|e| BindError::Bind(format!("unlink stale {path:?}: {e}")))?;
            }

            // The path exists but isn't a UDS at all — `connect(2)`
            // returns `ENOTSOCK`/`EOPNOTSUPP` (errno 38 on macOS,
            // surfaced as `ErrorKind::Uncategorized`). This is the
            // stale-file scenario the `ipc_stale_socket_is_cleaned`
            // test exercises: a previous run left a regular file at
            // the socket path. Safe to remove because nothing on the
            // system can be using it as a socket.
            Ok(Err(e)) if e.raw_os_error() == Some(libc::ENOTSOCK) => {
                std::fs::remove_file(path)
                    .map_err(|e| BindError::Bind(format!("unlink stale {path:?}: {e}")))?;
            }

            // Anything else — including `timeout` (Err(_)) and other
            // connect errors (permission denied, EAGAIN under fd
            // exhaustion, dangling-symlink NotFound) — could mean a
            // live engine is paged out or the FS is in an unusual
            // state. Refuse to steal rather than risk killing a
            // healthy engine's socket.
            Ok(Err(_)) | Err(_) => {
                return Err(BindError::EngineAlreadyRunning(path.to_path_buf()));
            }
        }
    }

    // Set umask to 0o077 around the bind so the inode is created with mode
    // 0o600 from the start, closing the window between bind() and chmod()
    // where a local user could connect(2) to the new socket.
    let listener = unsafe { with_strict_umask(|| UnixListener::bind(path)) }
        .map_err(|e| BindError::Bind(format!("bind {path:?}: {e}")))?;

    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        // Belt-and-braces — the umask above already set perms to 0o600, but
        // if set_permissions fails something is wrong with the FS. Drop the
        // listener and remove the socket file before returning so we don't
        // leave a permissive inode behind.
        drop(listener);
        let _ = std::fs::remove_file(path);
        return Err(BindError::Bind(format!("chmod {path:?}: {e}")));
    }
    Ok(listener)
}

#[derive(Debug)]
pub(super) enum BindError {
    EngineAlreadyRunning(PathBuf),
    Bind(String),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::EngineAlreadyRunning(p) => {
                write!(f, "engine already running at {}", p.display())
            }
            BindError::Bind(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for BindError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Save HOME, run `f`, restore HOME. Env vars are process-global so
    /// these tests must not run in parallel with each other; we rely on
    /// `cargo test`'s default thread scheduling and keep the critical
    /// section short. A single combined test sidesteps cross-test races.
    fn with_home<F: FnOnce()>(value: Option<&str>, f: F) {
        let saved = std::env::var_os("HOME");
        // SAFETY: env mutation is process-global. These tests must not run
        // concurrently with other tests touching HOME; we keep the critical
        // section short and restore on exit.
        unsafe {
            match value {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
        f();
        unsafe {
            match saved {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn default_socket_path_validates_home() {
        // Empty HOME is rejected.
        with_home(Some(""), || {
            let err = default_socket_path().expect_err("empty HOME must be rejected");
            assert!(err.contains("empty"), "got: {err}");
        });

        // Relative HOME is rejected.
        with_home(Some("relative/path"), || {
            let err = default_socket_path().expect_err("relative HOME must be rejected");
            assert!(err.contains("absolute"), "got: {err}");
        });

        // Absolute HOME is accepted and joined.
        with_home(Some("/tmp/fakehome"), || {
            let p = default_socket_path().expect("absolute HOME must be accepted");
            assert_eq!(
                p,
                PathBuf::from("/tmp/fakehome/Library/Application Support/panops/engine.sock")
            );
        });
    }
}
