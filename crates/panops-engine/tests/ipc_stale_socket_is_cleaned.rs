//! Slice 05 — pre-existing stale socket file is unlinked, server binds.

use std::os::unix::fs::FileTypeExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use tempfile::tempdir;

const BIN: &str = env!("CARGO_BIN_EXE_panops-engine");

#[test]
fn stale_socket_is_unlinked_and_rebound() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");

    std::fs::write(&socket, b"stale").unwrap();
    assert!(socket.exists());

    let mut child = Command::new(BIN)
        .args(["serve", "--socket"])
        .arg(&socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    let start = std::time::Instant::now();
    let mut bound = false;
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(meta) = std::fs::metadata(&socket) {
            if meta.file_type().is_socket() {
                bound = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let _ = child.wait();

    assert!(
        bound,
        "engine did not replace stale socket file with a live socket"
    );
}
