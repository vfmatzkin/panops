//! ScreenCaptureKit capture sidecar adapter. Spawns `panops-capture-mac`
//! as a child process and drives it over newline-delimited JSON-RPC on
//! stdio. Lazy spawn on first `start_capture`; the process is reused
//! across sessions.
//!
//! Unlike the ASR / LLM sidecars (one `transcribe` / `complete` line in,
//! one result line out — request/response), capture is a **stateful
//! session**: `start_capture` writes `capture.start` and reads a
//! `started` ack, the sidecar then records in the background until
//! `stop_capture` writes `capture.stop` and reads the finalized paths.
//! Everything else — the 16 MiB line cap, `jsonrpc` version check, id
//! pairing, `*slot = None` respawn-on-broken-pipe, and the `Drop` that
//! closes stdin then reaps the child — mirrors `whisperkit_asr.rs`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use panops_core::capture::{
    AudioSources, Capture, CaptureConfig, CaptureError, CaptureResult, CaptureSession,
    CaptureTarget, WindowInfo,
};
use serde::{Deserialize, Deserializer};

/// Reject sidecar response lines larger than 16 MiB. A capture result is
/// a few hundred bytes (two paths + a screenshot list); this cap is purely
/// a safety bound against a wedged sidecar emitting unbounded output.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// macOS live-capture adapter. Drives the `panops-capture-mac` sidecar.
pub struct ScreenCaptureKitCapture {
    binary: PathBuf,
    /// Extra environment handed to the spawned sidecar. Empty in
    /// production; the conformance tests use it to drive the fake
    /// sidecar's error modes without leaking test knobs into the
    /// adapter's control protocol.
    extra_env: Vec<(String, String)>,
    state: Mutex<Option<SidecarState>>,
    next_id: Mutex<u64>,
    /// Live capture sessions keyed by `meeting_id`. A `stop_capture` for a
    /// `meeting_id` absent here is `SessionNotFound` without ever touching
    /// the sidecar (the sidecar's own unknown-session error maps the same
    /// way for the rare wire-level case).
    sessions: Mutex<HashMap<String, ()>>,
}

struct SidecarState {
    /// `Option` so `Drop` can `take()` the child + close pipes before
    /// `kill()` / `wait()`. Always `Some` while the sidecar is alive.
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Drop for SidecarState {
    fn drop(&mut self) {
        // Closing stdin first lets the sidecar's readLine loop see EOF
        // (it stops any in-flight capture and exits cleanly). kill + wait
        // afterward catches the wedged case so we never leak a zombie when
        // `*slot = None` runs on an error path.
        drop(self.stdin.take());
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Generic JSON-RPC response envelope. `R` is the per-method result shape
/// (`StartedResult` for `capture.start`, `StopResult` for `capture.stop`).
#[derive(Deserialize)]
struct RpcResponse<R> {
    #[serde(rename = "jsonrpc", deserialize_with = "deserialize_jsonrpc_version")]
    _jsonrpc: (),
    /// `Option` so a sidecar parse-error response with `id: null`
    /// (JSON-RPC 2.0 §4) decodes instead of failing — we still surface it
    /// as an error because we can't pair it to a request. serde defaults a
    /// missing `Option` field to `None`, so no `#[serde(default)]` is
    /// needed (and on the generic `result` it would spuriously demand
    /// `R: Default`).
    id: Option<u64>,
    result: Option<R>,
    error: Option<RpcError>,
}

/// Reject responses whose `jsonrpc` field is anything but `"2.0"`. A silent
/// mismatch would hide framing-version drift between engine and sidecar.
fn deserialize_jsonrpc_version<'de, D>(de: D) -> Result<(), D::Error>
where
    D: Deserializer<'de>,
{
    let v = String::deserialize(de)?;
    if v != "2.0" {
        return Err(serde::de::Error::custom(format!(
            "unsupported jsonrpc version: {v}"
        )));
    }
    Ok(())
}

#[derive(Deserialize, Debug)]
struct RpcError {
    code: i32,
    message: String,
}

/// Result of `capture.start`.
#[derive(Deserialize)]
struct StartedResult {
    started_at_ms: u64,
}

/// Result of `capture.stop`. Each audio path is non-null exactly when its
/// source was captured.
#[derive(Deserialize)]
struct StopResult {
    #[serde(default)]
    system_audio_path: Option<String>,
    #[serde(default)]
    mic_audio_path: Option<String>,
    #[serde(default)]
    screenshot_paths: Vec<String>,
    #[serde(default)]
    duration_ms: u64,
}

#[derive(Deserialize)]
struct WindowInfoWire {
    window_id: u32,
    app_name: String,
    title: String,
}

impl ScreenCaptureKitCapture {
    /// Construct an adapter that spawns `binary` on first `start_capture`.
    pub fn new(binary: PathBuf) -> Self {
        Self::with_env(binary, Vec::new())
    }

    /// Construct an adapter that passes `extra_env` to the spawned sidecar.
    /// Production resolution uses [`Self::new`]; this entry point exists so
    /// the conformance suite can select a fake-sidecar error mode per
    /// adapter instance (no global env mutation, no cross-test bleed).
    pub fn with_env(binary: PathBuf, extra_env: Vec<(String, String)>) -> Self {
        Self {
            binary,
            extra_env,
            state: Mutex::new(None),
            next_id: Mutex::new(1),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn ensure_spawned(&self, slot: &mut Option<SidecarState>) -> Result<(), CaptureError> {
        if slot.is_some() {
            return Ok(());
        }
        tracing::info!(binary = %self.binary.display(), "spawning panops-capture-mac sidecar");
        let mut child = Command::new(&self.binary)
            .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Sidecar stderr passes through to the engine's stderr so
            // ScreenCaptureKit / TCC detail lands in Console.app — only an
            // opaque code crosses the wire (see `map_sidecar_error`).
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| CaptureError::Sidecar(format!("sidecar spawn: {e}")))?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        *slot = Some(SidecarState {
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
        });
        Ok(())
    }

    fn next_id(&self) -> Result<u64, CaptureError> {
        // Map a poisoned next_id mutex to a typed error instead of
        // panicking on a future call, per panops's "no panics in
        // production" discipline. The lock is held only during an integer
        // increment, so poison is exceedingly unlikely.
        let mut id = self
            .next_id
            .lock()
            .map_err(|e| CaptureError::Sidecar(format!("next_id mutex poisoned: {e}")))?;
        let v = *id;
        *id += 1;
        Ok(v)
    }

    /// Send one control line and read one id-paired response. Generic over
    /// the result shape so `capture.start` and `capture.stop` share the
    /// framing discipline verbatim. Any stdio failure clears the sidecar
    /// slot so the next call respawns cleanly.
    fn call<R: serde::de::DeserializeOwned>(
        &self,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<R, CaptureError> {
        let mut slot = self
            .state
            .lock()
            .map_err(|e| CaptureError::Sidecar(format!("sidecar state mutex poisoned: {e}")))?;
        self.ensure_spawned(&mut slot)?;

        let id = self.next_id()?;
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": [params],
        });
        let body =
            serde_json::to_vec(&req).map_err(|e| CaptureError::Sidecar(format!("encode: {e}")))?;

        // Clear the slot on any stdio failure so the next call respawns. A
        // BrokenPipe (sidecar died mid-write) without this would leave the
        // slot populated and every subsequent call would hit the dead pipe.
        let write_result = {
            let state = slot.as_mut().expect("just spawned");
            let stdin = state.stdin.as_mut().expect("stdin live while spawned");
            stdin
                .write_all(&body)
                .and_then(|()| stdin.write_all(b"\n"))
                .and_then(|()| stdin.flush())
        };
        if let Err(e) = write_result {
            *slot = None;
            return Err(CaptureError::Sidecar(format!("stdio write: {e}")));
        }

        // Bound the response read against a wedged sidecar emitting an
        // unbounded line. `take(MAX_RESPONSE_BYTES)` + `read_until(b'\n')`
        // gives a strict cap; a missing trailing newline means the line was
        // truncated and framing is lost.
        let mut buf: Vec<u8> = Vec::new();
        let read_result = {
            let state = slot.as_mut().expect("still spawned");
            (&mut state.stdout)
                .take(MAX_RESPONSE_BYTES)
                .read_until(b'\n', &mut buf)
        };
        let n = match read_result {
            Ok(n) => n,
            Err(e) => {
                *slot = None;
                return Err(CaptureError::Sidecar(format!("stdio read: {e}")));
            }
        };
        if n == 0 {
            // Sidecar exited; drop the state so the next call respawns.
            *slot = None;
            return Err(CaptureError::Sidecar(
                "sidecar exited before responding".into(),
            ));
        }
        if buf.last() != Some(&b'\n') {
            // No trailing newline: either the sidecar died mid-line or the
            // line hit `MAX_RESPONSE_BYTES`. Distinguish so the message
            // points at the right failure mode during debugging.
            *slot = None;
            let msg = if (n as u64) >= MAX_RESPONSE_BYTES {
                format!("sidecar response exceeded {MAX_RESPONSE_BYTES} bytes without newline")
            } else {
                format!("sidecar response truncated at {n} bytes (EOF mid-line)")
            };
            return Err(CaptureError::Sidecar(msg));
        }
        let line = std::str::from_utf8(&buf)
            .map_err(|e| CaptureError::Sidecar(format!("response not utf-8: {e}")))?;
        let resp: RpcResponse<R> = serde_json::from_str(line.trim_end()).map_err(|e| {
            *slot = None;
            CaptureError::Sidecar(format!("decode: {e}"))
        })?;
        // Validate id pairing. A `None` id is a parse-error response
        // (JSON-RPC 2.0 §4); a mismatched id means we lost framing. Either
        // way the stream is corrupt and the sidecar must respawn.
        match resp.id {
            Some(rid) if rid == id => {}
            Some(rid) => {
                *slot = None;
                return Err(CaptureError::Sidecar(format!(
                    "JSON-RPC id mismatch: expected {id}, got {rid}"
                )));
            }
            None => {
                *slot = None;
                let detail = resp
                    .error
                    .as_ref()
                    .map(|e| format!(" (sidecar code {})", e.code))
                    .unwrap_or_default();
                return Err(CaptureError::Sidecar(format!(
                    "sidecar returned null id{detail}"
                )));
            }
        }
        if let Some(err) = resp.error {
            // Log full sidecar detail (TCC state, ScreenCaptureKit errors)
            // locally; surface an opaque, code-derived `CaptureError` over
            // the wire so client-facing IPC doesn't echo sidecar internals.
            tracing::error!(
                code = err.code,
                message = %err.message,
                "panops-capture-mac sidecar returned error"
            );
            return Err(map_sidecar_error(err.code));
        }
        resp.result
            .ok_or_else(|| CaptureError::Sidecar("response missing result".into()))
    }
}

impl Capture for ScreenCaptureKitCapture {
    fn list_windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
        let output = Command::new(&self.binary)
            .arg("--list-windows")
            .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| CaptureError::Sidecar(format!("list-windows spawn: {e}")))?;
        if !output.status.success() {
            return Err(CaptureError::Sidecar(format!(
                "list-windows exited with status {}",
                output.status
            )));
        }
        let windows: Vec<WindowInfoWire> = serde_json::from_slice(&output.stdout)
            .map_err(|e| CaptureError::Sidecar(format!("list-windows decode: {e}")))?;
        Ok(windows
            .into_iter()
            .map(|w| WindowInfo {
                window_id: w.window_id,
                app_name: w.app_name,
                title: w.title,
            })
            .collect())
    }

    fn start_capture(
        &self,
        meeting_id: &str,
        meeting_dir: &Path,
        config: &CaptureConfig,
    ) -> Result<CaptureSession, CaptureError> {
        let (system_audio_path, mic_audio_path) = track_paths(meeting_dir, config.audio_sources);
        let params = serde_json::json!({
            "meeting_id": meeting_id,
            "system_audio_path": system_audio_path,
            "mic_audio_path": mic_audio_path,
            "screenshots_dir": meeting_dir.join("screenshots").display().to_string(),
            "audio_sources": audio_sources_str(config.audio_sources),
            "record_video": config.record_video,
            "video_path": meeting_dir.join("recording.mov").display().to_string(),
            "screenshot_interval_ms": config.screenshot_interval_ms,
            "screenshot_threshold": config.screenshot_threshold,
            "capture_target": capture_target_json(&config.capture_target),
        });
        let started: StartedResult = self.call("capture.start", params)?;
        // Insert into the sessions map AFTER successful sidecar call.
        // If the lock is poisoned after a successful start, make a best-effort
        // stop call to avoid an orphaned recording before returning the error.
        match self.sessions.lock() {
            Ok(mut sessions) => {
                sessions.insert(meeting_id.to_string(), ());
            }
            Err(e) => {
                // Poisoned mutex: best-effort stop to avoid orphaned recording
                // Note: type inference fails for call() here; StopResult is safe since
                // we're just trying to stop the sidecar - any response is acceptable.
                let _ = self.call::<StopResult>(
                    "capture.stop",
                    serde_json::json!({ "meeting_id": meeting_id }),
                );
                return Err(CaptureError::Sidecar(format!(
                    "sessions mutex poisoned: {e}"
                )));
            }
        }
        Ok(CaptureSession {
            meeting_id: meeting_id.to_string(),
            started_at_ms: started.started_at_ms,
            capture_target: config.capture_target.clone(),
        })
    }

    fn stop_capture(&self, session: &CaptureSession) -> Result<CaptureResult, CaptureError> {
        // An unknown session is `SessionNotFound` without ever talking to
        // the sidecar — the live-session map is the source of truth.
        // Order matters: call the sidecar FIRST, then remove from the map.
        // If the sidecar call fails, the recording is not orphaned.
        let params = serde_json::json!({ "meeting_id": session.meeting_id });

        // Check session existence and prepare return value first, but don't remove yet
        let capture_result = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|e| CaptureError::Sidecar(format!("sessions mutex poisoned: {e}")))?;
            if sessions.get(&session.meeting_id).is_none() {
                return Err(CaptureError::SessionNotFound(session.meeting_id.clone()));
            }
            // Call the sidecar BEFORE removing from the map
            let r: StopResult = self.call("capture.stop", params)?;
            // Now remove the session from the map
            sessions.remove(&session.meeting_id);
            CaptureResult {
                system_audio_path: r.system_audio_path.map(PathBuf::from),
                mic_audio_path: r.mic_audio_path.map(PathBuf::from),
                screenshot_paths: r.screenshot_paths.into_iter().map(PathBuf::from).collect(),
                duration_ms: r.duration_ms,
            }
        };
        Ok(capture_result)
    }
}

impl Drop for ScreenCaptureKitCapture {
    fn drop(&mut self) {
        // `SidecarState`'s own `Drop` closes stdin and reaps the child; we
        // just take the slot here so it runs. Lock poisoning is recovered
        // via `into_inner()` so the child is always reaped on every path.
        let mut slot = self.state.lock().unwrap_or_else(|p| p.into_inner());
        slot.take();
    }
}

/// Derive the two per-track WAV paths under `meeting_dir`, returning `None`
/// for any source `sources` did not request. A null path is the wire
/// signal for "do not capture that source".
fn track_paths(meeting_dir: &Path, sources: AudioSources) -> (Option<String>, Option<String>) {
    let want_system = !matches!(sources, AudioSources::MicOnly);
    let want_mic = !matches!(sources, AudioSources::SystemOnly);
    let system = want_system.then(|| meeting_dir.join("system.wav").display().to_string());
    let mic = want_mic.then(|| meeting_dir.join("mic.wav").display().to_string());
    (system, mic)
}

/// Map `AudioSources` to its control-protocol wire string.
fn audio_sources_str(sources: AudioSources) -> &'static str {
    match sources {
        AudioSources::SystemOnly => "system_only",
        AudioSources::MicOnly => "mic_only",
        AudioSources::SystemAndMic => "system_and_mic",
    }
}

/// Map `CaptureTarget` to the sidecar control-protocol shape.
fn capture_target_json(target: &CaptureTarget) -> serde_json::Value {
    match target {
        CaptureTarget::Display { display_id } => {
            serde_json::json!({ "kind": "display", "display_id": display_id })
        }
        CaptureTarget::Window { window_id } => {
            serde_json::json!({ "kind": "window", "window_id": window_id })
        }
        CaptureTarget::App { bundle_id } => {
            serde_json::json!({ "kind": "app", "bundle_id": bundle_id })
        }
        CaptureTarget::Region {
            display_id,
            x,
            y,
            w,
            h,
        } => serde_json::json!({
            "kind": "region",
            "display_id": display_id,
            "x": x,
            "y": y,
            "w": w,
            "h": h,
        }),
    }
}

/// Map a sidecar JSON-RPC error code to an opaque `CaptureError`. Full
/// detail is logged via `tracing`; only the code shapes the variant.
fn map_sidecar_error(code: i32) -> CaptureError {
    match code {
        -32001 => CaptureError::PermissionDenied("screen recording".into()),
        -32002 => CaptureError::PermissionDenied("microphone".into()),
        -32004 => CaptureError::SessionNotFound("sidecar reported unknown session".into()),
        _ => CaptureError::Sidecar(format!("sidecar error (code {code})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_id_monotonic() {
        let cap = ScreenCaptureKitCapture::new(PathBuf::from("/nonexistent"));
        let ids: Vec<u64> = (0..5).map(|_| cap.next_id().expect("next_id")).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn track_paths_per_source() {
        let dir = Path::new("/tmp/meeting");

        let (s, m) = track_paths(dir, AudioSources::SystemAndMic);
        assert_eq!(s.as_deref(), Some("/tmp/meeting/system.wav"));
        assert_eq!(m.as_deref(), Some("/tmp/meeting/mic.wav"));

        let (s, m) = track_paths(dir, AudioSources::SystemOnly);
        assert_eq!(s.as_deref(), Some("/tmp/meeting/system.wav"));
        assert_eq!(m, None);

        let (s, m) = track_paths(dir, AudioSources::MicOnly);
        assert_eq!(s, None);
        assert_eq!(m.as_deref(), Some("/tmp/meeting/mic.wav"));
    }

    #[test]
    fn audio_sources_wire_strings() {
        assert_eq!(audio_sources_str(AudioSources::SystemOnly), "system_only");
        assert_eq!(audio_sources_str(AudioSources::MicOnly), "mic_only");
        assert_eq!(
            audio_sources_str(AudioSources::SystemAndMic),
            "system_and_mic"
        );
    }

    #[test]
    fn capture_target_json_uses_pinned_shape() {
        assert_eq!(
            capture_target_json(&CaptureTarget::Display { display_id: 0 }),
            serde_json::json!({ "kind": "display", "display_id": 0 })
        );
        assert_eq!(
            capture_target_json(&CaptureTarget::Window { window_id: 42 }),
            serde_json::json!({ "kind": "window", "window_id": 42 })
        );
    }

    #[test]
    fn sidecar_error_codes_map_to_variants() {
        assert!(matches!(
            map_sidecar_error(-32001),
            CaptureError::PermissionDenied(_)
        ));
        assert!(matches!(
            map_sidecar_error(-32002),
            CaptureError::PermissionDenied(_)
        ));
        assert!(matches!(
            map_sidecar_error(-32004),
            CaptureError::SessionNotFound(_)
        ));
        assert!(matches!(map_sidecar_error(-1), CaptureError::Sidecar(_)));
    }

    #[test]
    fn unknown_session_short_circuits_without_sidecar() {
        // No sidecar process is spawnable at this path; stop on an unknown
        // session must still return SessionNotFound (the live-session map
        // check happens before any stdio).
        let cap = ScreenCaptureKitCapture::new(PathBuf::from("/nonexistent"));
        let session = CaptureSession {
            meeting_id: "never_started".into(),
            started_at_ms: 0,
            capture_target: CaptureTarget::Display { display_id: 0 },
        };
        let err = cap.stop_capture(&session).expect_err("should fail");
        assert!(matches!(err, CaptureError::SessionNotFound(id) if id == "never_started"));
    }
}
