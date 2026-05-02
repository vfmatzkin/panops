//! Slice 05 — a second `serve` on a live socket exits non-zero.

mod common;

use std::process::{Command, Stdio};
use std::time::Duration;

use tempfile::tempdir;

use common::wait_with_timeout;

const BIN: &str = env!("CARGO_BIN_EXE_panops-engine");

#[test]
fn second_serve_refuses_when_engine_already_running() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");

    let mut first = Command::new(BIN)
        .args(["serve", "--socket"])
        .arg(&socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn first");

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if socket.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let second = Command::new(BIN)
        .args(["serve", "--socket"])
        .arg(&socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn second");

    assert!(!second.status.success(), "second serve should fail");
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("engine already running"),
        "stderr did not mention 'engine already running': {stderr}"
    );

    unsafe {
        libc::kill(first.id() as i32, libc::SIGTERM);
    }
    let _ = wait_with_timeout(&mut first, Duration::from_secs(5));
}
