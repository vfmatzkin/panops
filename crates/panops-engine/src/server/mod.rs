//! IPC server entry point. Owns the tokio runtimes and binds the UDS.
//!
//! See `docs/superpowers/specs/2026-05-02-slice-05-ipc-design.md`.

use std::path::PathBuf;

pub fn run_serve(_socket: Option<PathBuf>) -> Result<(), (u8, String)> {
    // Stub — Tasks 3G, 4I, 5K replace this body.
    Err((
        1,
        "panops-engine serve: not implemented yet (slice 05 in progress)".to_string(),
    ))
}
