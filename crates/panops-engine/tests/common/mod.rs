//! Shared helpers for the slice-05 IPC integration tests.
//!
//! Cargo compiles every top-level file under `tests/` as its own
//! integration-test crate; using a directory module (`common/mod.rs`)
//! instead of a flat `tests/common.rs` keeps cargo from compiling this
//! file standalone and warning about unused helpers when a given test
//! only uses one of them. Each test that needs these adds
//! `mod common;` and imports from the resulting module.
//!
//! `#[allow(dead_code)]` is on the module rather than each fn because
//! a given test binary only pulls in a subset of these helpers, and
//! the rest land as `unused` per integration-test compilation.

#![allow(dead_code)]

pub mod notes_pipeline;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::Arc;
use std::time::{Duration, Instant};

use panops_core::conformance::fakes::InMemoryStorage;
use panops_core::storage::Storage;
use tempfile::TempDir;
use tokio::net::UnixStream;

const ENGINE_BIN: &str = env!("CARGO_BIN_EXE_panops-engine");

/// Returns a `Command` for `panops-engine serve` with `--socket` and
/// `--data-dir` pre-filled from the given `TempDir`. The socket path
/// is `<dir>/engine.sock`. Tests MUST hold the `TempDir` until after
/// the spawned engine exits — otherwise the dir is reaped while the
/// engine still expects it.
///
/// Centralizing the subprocess builder here prevents regression of
/// issue #228: a test that spawns the engine without `--data-dir`
/// opens the user's REAL registry at `~/Library/Application Support/panops/`.
pub fn engine_serve_command(dir: &TempDir) -> Command {
    let socket = dir.path().join("engine.sock");
    let mut cmd = Command::new(ENGINE_BIN);
    cmd.args(["serve", "--socket"])
        .arg(&socket)
        .arg("--data-dir")
        .arg(dir.path());
    cmd
}

/// Returns a `(TempDir, Arc<dyn Storage>, PathBuf)` triple for tests.
/// The caller MUST hold the `TempDir` until after the test ends so the
/// dir isn't reaped while the engine still expects it. The `PathBuf`
/// is the same `tmp.path()`, copied to free the borrow.
pub fn tempdir_storage() -> (TempDir, Arc<dyn Storage>, PathBuf) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let path = tmp.path().to_owned();
    let storage: Arc<dyn Storage> = Arc::new(InMemoryStorage::new());
    (tmp, storage, path)
}

/// Poll `child.try_wait()` until it exits or `dur` elapses, then SIGKILL
/// and reap. Used by tests that send SIGTERM to the engine and need a
/// hard upper bound — without this they hang the whole test binary if
/// shutdown ever regresses.
pub fn wait_with_timeout(child: &mut Child, dur: Duration) -> std::io::Result<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(s) = child.try_wait()? {
            return Ok(s);
        }
        if start.elapsed() > dur {
            let _ = child.kill();
            return child.wait();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Block until the engine's UDS at `path` is connectable, or panic
/// after 5 s. Existence alone isn't enough: the file appears slightly
/// before `accept` is wired, so we connect-and-drop to confirm the
/// listener is actually serving.
pub async fn wait_for_socket(path: &Path) {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if path.exists() && UnixStream::connect(path).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("socket never became connectable: {path:?}");
}

/// Open a jsonrpsee WebSocket client over the engine's UDS. The
/// `ws://localhost` URL is a placeholder — jsonrpsee uses the
/// pre-built stream instead of dialing it.
pub async fn uds_ws_client(path: &Path) -> jsonrpsee::ws_client::WsClient {
    let stream = UnixStream::connect(path).await.expect("connect uds");
    jsonrpsee::ws_client::WsClientBuilder::default()
        .build_with_stream("ws://localhost", stream)
        .await
        .expect("build ws client over uds")
}
