//! Slice 05 — socket file has mode 0600 immediately after bind.

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use tempfile::tempdir;

const BIN: &str = env!("CARGO_BIN_EXE_panops-engine");

#[test]
fn socket_has_mode_0600() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");

    let mut child = Command::new(BIN)
        .args(["serve", "--socket"])
        .arg(&socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if socket.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let meta = std::fs::metadata(&socket).expect("stat socket");
    let mode = meta.permissions().mode() & 0o777;

    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let _ = child.wait();

    assert_eq!(mode, 0o600, "socket mode is {mode:o}, expected 0o600");
}
