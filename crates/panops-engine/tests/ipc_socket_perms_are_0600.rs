//! Slice 05 — socket file has mode 0600 immediately after bind.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use tempfile::tempdir;

use common::wait_with_timeout;

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

    // Poll until perms have actually settled. There's a brief window
    // between `bind(2)` creating the inode and `chmod(2)` (or umask)
    // applying 0o600 — a pure existence check could observe the
    // pre-chmod mode and flake. Loop until the mode matches OR the
    // 5s budget runs out.
    let start = std::time::Instant::now();
    let mut observed_mode: u32 = 0;
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(meta) = std::fs::metadata(&socket) {
            let mode = meta.permissions().mode() & 0o777;
            if mode == 0o600 {
                observed_mode = mode;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let _ = wait_with_timeout(&mut child, Duration::from_secs(5));

    assert_eq!(
        observed_mode, 0o600,
        "socket mode never settled to 0o600 (last observed: {observed_mode:o})"
    );
}
