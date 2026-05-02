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
pub fn default_socket_path() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    Ok(PathBuf::from(home).join("Library/Application Support/panops/engine.sock"))
}

/// Bind a `UnixListener` at `path` after probing for a live engine and
/// removing any stale socket file. Sets `0600` perms on the socket.
pub async fn bind_with_lifecycle(path: &Path) -> Result<UnixListener, BindError> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| BindError::Bind(format!("create parent {parent:?}: {e}")))?;
            // Tighten parent perms (best-effort).
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }

    if path.exists() {
        // Probe for a live listener with a short timeout.
        let probe = timeout(Duration::from_millis(250), UnixStream::connect(path)).await;
        match probe {
            Ok(Ok(_)) => return Err(BindError::EngineAlreadyRunning(path.to_path_buf())),
            Ok(Err(_)) | Err(_) => {
                std::fs::remove_file(path)
                    .map_err(|e| BindError::Bind(format!("unlink stale {path:?}: {e}")))?;
            }
        }
    }

    let listener =
        UnixListener::bind(path).map_err(|e| BindError::Bind(format!("bind {path:?}: {e}")))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| BindError::Bind(format!("chmod {path:?}: {e}")))?;
    Ok(listener)
}

#[derive(Debug)]
pub enum BindError {
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
