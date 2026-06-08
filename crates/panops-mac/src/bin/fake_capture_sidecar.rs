//! Test double for the `panops-capture-mac` control protocol. NOT shipped
//! — declared as a `[[bin]]` so the adapter conformance test can locate it
//! via `CARGO_BIN_EXE_fake-capture-sidecar` and exercise the spawn / stdio
//! / respawn machinery without ScreenCaptureKit, a TCC grant, or a screen.
//!
//! It speaks the start/ack + stop/result protocol with canned responses:
//! `capture.start` records the requested track paths and acks; `capture.stop`
//! writes a tiny valid 16 kHz mono WAV to each requested path and replies
//! with the paths. `PANOPS_FAKE_SIDECAR_MODE` selects error paths:
//! `drop_pipe` exits before acking a start; `unknown_session` returns the
//! `-32004` error on stop.
//!
//! On non-macOS targets this compiles to an empty `main` (the macOS-only
//! `hound` / `serde_json` deps are gated out) so `cargo build --workspace`
//! stays green everywhere — it is only ever run by the macOS-gated test.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::io::{BufRead, Write};

    pub fn run() {
        if std::env::args().any(|arg| arg == "--list-windows") {
            println!(
                "{}",
                serde_json::json!([
                    { "window_id": 101u32, "app_name": "Safari", "title": "Panops Fixture Window" },
                    { "window_id": 202u32, "app_name": "Notes", "title": "Meeting Notes" }
                ])
            );
            return;
        }

        let mode = std::env::var("PANOPS_FAKE_SIDECAR_MODE").unwrap_or_default();
        let stdin = std::io::stdin();
        let mut out = std::io::stdout();
        // The single active session's (system, mic, video) paths. The
        // conformance suite is strictly sequential (start → stop), so one
        // slot suffices.
        let mut active: (serde_json::Value, serde_json::Value, serde_json::Value) = (
            serde_json::Value::Null,
            serde_json::Value::Null,
            serde_json::Value::Null,
        );

        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(req) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let id = req["id"].clone();
            let method = req["method"].as_str().unwrap_or("");
            let params = &req["params"][0];
            match method {
                "capture.start" => {
                    if mode == "drop_pipe" {
                        // Die before acking: the adapter sees EOF and must
                        // clear its slot so the next call respawns.
                        std::process::exit(0);
                    }
                    active = (
                        params["system_audio_path"].clone(),
                        params["mic_audio_path"].clone(),
                        if params["record_video"].as_bool().unwrap_or(false) {
                            params["video_path"].clone()
                        } else {
                            serde_json::Value::Null
                        },
                    );
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "started_at_ms": 1_700_000_000_000u64 },
                    });
                    reply(&mut out, &resp);
                }
                "capture.stop" => {
                    if mode == "unknown_session" {
                        let resp = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32004, "message": "session not found" },
                        });
                        reply(&mut out, &resp);
                        continue;
                    }
                    let (sys, mic, video) = std::mem::replace(
                        &mut active,
                        (
                            serde_json::Value::Null,
                            serde_json::Value::Null,
                            serde_json::Value::Null,
                        ),
                    );
                    write_video(&video);
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "system_audio_path": write_track(&sys),
                            "mic_audio_path": write_track(&mic),
                            "screenshot_paths": [],
                            "duration_ms": 1000u64,
                        },
                    });
                    reply(&mut out, &resp);
                }
                _ => {}
            }
        }
    }

    fn reply(out: &mut std::io::Stdout, resp: &serde_json::Value) {
        // stdout is block-buffered over a pipe; flush so the adapter's
        // blocking read sees the line immediately.
        writeln!(out, "{resp}").expect("write response");
        out.flush().expect("flush response");
    }

    /// Write a 1-second 16 kHz mono silent WAV to `path` (16 000 samples →
    /// the conformance duration check) and echo the path back, or `null`
    /// when the source was not requested.
    fn write_track(path: &serde_json::Value) -> serde_json::Value {
        let Some(p) = path.as_str() else {
            return serde_json::Value::Null;
        };
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(p, spec).expect("create wav");
        for _ in 0..16_000 {
            w.write_sample(0i16).expect("write sample");
        }
        w.finalize().expect("finalize wav");
        serde_json::Value::String(p.to_string())
    }

    fn write_video(path: &serde_json::Value) {
        let Some(p) = path.as_str() else {
            return;
        };
        std::fs::write(
            p,
            b"\x00\x00\x00\x14ftypqt  \x00\x00\x00\x00qt  \x00\x00\x00\x08mdat",
        )
        .expect("write fake mov");
    }
}
