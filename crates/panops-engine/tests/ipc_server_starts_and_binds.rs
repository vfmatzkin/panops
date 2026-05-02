//! Slice 05 — server starts, socket exists, binary exits cleanly on signal.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio::net::UnixStream;

const BIN: &str = env!("CARGO_BIN_EXE_panops-engine");

fn wait_for_socket(path: &std::path::Path, deadline: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn server_binds_socket_and_shuts_down_on_sigterm() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("engine.sock");

    let mut child = Command::new(BIN)
        .args(["serve", "--socket"])
        .arg(&socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn engine");

    assert!(
        wait_for_socket(&socket, Duration::from_secs(5)),
        "socket did not appear within 5s"
    );

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let _ = UnixStream::connect(&socket)
            .await
            .expect("connect to live socket");
    });

    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let status = wait_with_timeout(&mut child, Duration::from_secs(5)).expect("wait child");
    assert!(status.success(), "engine did not exit cleanly: {status:?}");
    assert!(!socket.exists(), "socket file persisted after shutdown");
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    dur: Duration,
) -> std::io::Result<std::process::ExitStatus> {
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
