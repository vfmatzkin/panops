//! FoundationModels LLM sidecar adapter. Spawns `panops-llm-mac` as a
//! child process and communicates via newline-delimited JSON-RPC over
//! stdio. Lazy spawn on first probe/complete; reused across calls to
//! amortize FoundationModels session setup.

use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_short};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use panops_core::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};
use serde::{Deserialize, Deserializer, Serialize};

/// Reject sidecar response lines larger than 16 MiB. Guided notes JSON
/// is expected to be small; this is a safety bound against a wedged
/// sidecar emitting an unbounded line.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
/// Startup probes run before the IPC socket binds; keep this short so a
/// wedged sidecar cannot prevent `serve` from starting and falling back.
const PROBE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
/// Completion can legitimately take longer, but still must be bounded so a
/// wedged model/sidecar does not poison the adapter forever.
const COMPLETE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const O_NONBLOCK: c_int = 0x0004;
const POLLIN: c_short = 0x0001;
const POLLNVAL: c_short = 0x0020;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

unsafe extern "C" {
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn poll(fds: *mut PollFd, nfds: u32, timeout: c_int) -> c_int;
}

pub struct FoundationLlm {
    binary: PathBuf,
    state: Mutex<Option<SidecarState>>,
    next_id: Mutex<u64>,
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
        // and exit cleanly. kill + wait afterward catches the wedged
        // case so we never leak a zombie when `*slot = None` runs on
        // an error path.
        drop(self.stdin.take());
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProbeResult {
    pub available: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Serialize)]
struct ProbeParams {}

#[derive(Serialize)]
struct CompleteParams<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    user: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema: Option<&'a serde_json::Value>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize)]
struct RpcRequest<P> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: P,
}

#[derive(Deserialize)]
struct RpcResponse<R> {
    #[serde(rename = "jsonrpc", deserialize_with = "deserialize_jsonrpc_version")]
    _jsonrpc: (),
    /// `Option` so a sidecar parse-error response with `id: null`
    /// (JSON-RPC 2.0 §4) decodes instead of failing — we still
    /// surface it as an error because we can't pair it to a request.
    id: Option<u64>,
    result: Option<R>,
    error: Option<RpcError>,
}

#[derive(Deserialize, Debug)]
struct RpcError {
    code: i32,
    message: String,
}

#[derive(Deserialize)]
struct CompleteResult {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    json: Option<serde_json::Value>,
}

/// Reject responses whose `jsonrpc` field is anything but `"2.0"`. A
/// silent mismatch would hide framing-version drift between the engine
/// and the sidecar.
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

impl FoundationLlm {
    pub fn new(binary: PathBuf) -> Self {
        Self {
            binary,
            state: Mutex::new(None),
            next_id: Mutex::new(1),
        }
    }

    /// Probe `SystemLanguageModel.availability` through the sidecar.
    /// Called by the engine resolver at startup and by tests; keeps the
    /// spawned process alive so a subsequent `complete` reuses it.
    pub fn probe(&self) -> Result<ProbeResult, LlmError> {
        self.send_request("probe", ProbeParams {})
    }

    fn ensure_spawned(&self, slot: &mut Option<SidecarState>) -> Result<(), LlmError> {
        if slot.is_some() {
            return Ok(());
        }
        tracing::info!(binary = %self.binary.display(), "spawning panops-llm-mac sidecar");
        let mut child = Command::new(&self.binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Sidecar's stderr passes through to engine's stderr so
            // FoundationModels availability/errors land in Console.app.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                tracing::warn!(
                    binary = %self.binary.display(),
                    error = %e,
                    "panops-llm-mac sidecar spawn failed"
                );
                LlmError::Provider("sidecar spawn failed".into())
            })?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        set_nonblocking(&stdout).map_err(|e| {
            tracing::warn!(
                binary = %self.binary.display(),
                error = %e,
                "panops-llm-mac sidecar stdout nonblocking setup failed"
            );
            LlmError::Provider("sidecar setup failed".into())
        })?;
        *slot = Some(SidecarState {
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
        });
        Ok(())
    }

    fn next_id(&self) -> Result<u64, LlmError> {
        let mut id = self
            .next_id
            .lock()
            .map_err(|e| LlmError::Provider(format!("next_id mutex poisoned: {e}")))?;
        let v = *id;
        *id += 1;
        Ok(v)
    }

    fn send_request<P, R>(&self, method: &'static str, params: P) -> Result<R, LlmError>
    where
        P: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        let mut slot = self
            .state
            .lock()
            .map_err(|e| LlmError::Provider(format!("sidecar state mutex poisoned: {e}")))?;
        self.ensure_spawned(&mut slot)?;

        let id = self.next_id()?;
        let req = RpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };
        let body =
            serde_json::to_vec(&req).map_err(|e| LlmError::Provider(format!("encode: {e}")))?;

        // Clear the sidecar state on any stdio failure so the next call
        // respawns cleanly. A BrokenPipe (sidecar died mid-write) without
        // this would leave the slot populated and every subsequent call
        // would fail on the same dead pipe.
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
            return Err(LlmError::Provider(format!("stdio write: {e}")));
        }

        // Bound the response read by size AND by deadline. The probe path is
        // exercised before the IPC socket binds, so timeout must surface as a
        // provider error and let the resolver fall back instead of hanging
        // `serve` forever.
        let mut buf: Vec<u8> = Vec::new();
        let timeout = response_timeout(method);
        let read_result = {
            let state = slot.as_mut().expect("still spawned");
            read_response_line(&mut state.stdout, &mut buf, timeout)
        };
        let n = match read_result {
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::TimedOut => {
                *slot = None;
                return Err(LlmError::Provider(format!(
                    "sidecar response timed out after {}s",
                    timeout.as_secs()
                )));
            }
            Err(e) => {
                *slot = None;
                return Err(LlmError::Provider(format!("stdio read: {e}")));
            }
        };
        if n == 0 {
            *slot = None;
            return Err(LlmError::Provider(
                "sidecar exited before responding".into(),
            ));
        }
        if buf.last() != Some(&b'\n') {
            *slot = None;
            let msg = if (n as u64) >= MAX_RESPONSE_BYTES {
                format!("sidecar response exceeded {MAX_RESPONSE_BYTES} bytes without newline")
            } else {
                format!("sidecar response truncated at {n} bytes (EOF mid-line)")
            };
            return Err(LlmError::Provider(msg));
        }
        let line = std::str::from_utf8(&buf)
            .map_err(|e| LlmError::Provider(format!("response not utf-8: {e}")))?;
        let resp: RpcResponse<R> = serde_json::from_str(line.trim_end()).map_err(|e| {
            *slot = None;
            LlmError::Provider(format!("decode: {e}"))
        })?;
        match resp.id {
            Some(rid) if rid == id => {}
            Some(rid) => {
                *slot = None;
                return Err(LlmError::Provider(format!(
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
                return Err(LlmError::Provider(format!(
                    "sidecar returned null id{detail}"
                )));
            }
        }
        if let Some(err) = resp.error {
            tracing::error!(
                code = err.code,
                message = %err.message,
                "panops-llm-mac sidecar returned error"
            );
            return match err.code {
                -32001 | -32602 => Err(LlmError::InvalidSchema {
                    expected: "valid JSON Schema convertible to FoundationModels".into(),
                    got: format!("sidecar error (code {})", err.code),
                }),
                -32002 => Err(LlmError::EmptyResponse),
                _ => Err(LlmError::Provider(format!(
                    "sidecar error (code {})",
                    err.code
                ))),
            };
        }
        resp.result
            .ok_or_else(|| LlmError::Provider("response missing result".into()))
    }
}

fn response_timeout(method: &str) -> Duration {
    if method == "probe" || method == "llm.probe" {
        PROBE_RESPONSE_TIMEOUT
    } else {
        COMPLETE_RESPONSE_TIMEOUT
    }
}

fn set_nonblocking(stdout: &ChildStdout) -> io::Result<()> {
    let fd = stdout.as_raw_fd();
    // SAFETY: `fd` is a live stdout pipe owned by `ChildStdout`. `fcntl` does
    // not take ownership and only updates descriptor flags.
    let flags = unsafe { fcntl(fd, F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Same live fd; OR-ing O_NONBLOCK preserves the existing flags.
    let rc = unsafe { fcntl(fd, F_SETFL, flags | O_NONBLOCK) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn read_response_line(
    stdout: &mut BufReader<ChildStdout>,
    buf: &mut Vec<u8>,
    timeout: Duration,
) -> io::Result<usize> {
    let deadline = Instant::now() + timeout;
    loop {
        match stdout.fill_buf() {
            Ok([]) if buf.is_empty() => return Ok(0),
            Ok([]) => return Ok(buf.len()),
            Ok(available) => {
                let consumed = if let Some(pos) = available.iter().position(|b| *b == b'\n') {
                    let end = pos + 1;
                    buf.extend_from_slice(&available[..end]);
                    end
                } else {
                    buf.extend_from_slice(available);
                    available.len()
                };
                stdout.consume(consumed);
                if buf.len() as u64 >= MAX_RESPONSE_BYTES {
                    return Ok(buf.len());
                }
                if buf.last() == Some(&b'\n') {
                    return Ok(buf.len());
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                wait_readable(stdout.get_ref().as_raw_fd(), deadline)?;
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

fn wait_readable(fd: i32, deadline: Instant) -> io::Result<()> {
    let now = Instant::now();
    if now >= deadline {
        return Err(io::Error::new(
            ErrorKind::TimedOut,
            "sidecar response timed out",
        ));
    }
    let remaining = deadline.saturating_duration_since(now);
    let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
    let mut fds = PollFd {
        fd,
        events: POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: `fds` points to one initialized pollfd for a live pipe fd;
        // poll does not retain the pointer after returning.
        let rc = unsafe { poll(&mut fds, 1, timeout_ms) };
        if rc > 0 {
            if fds.revents & POLLNVAL != 0 {
                return Err(io::Error::new(
                    ErrorKind::BrokenPipe,
                    "sidecar stdout descriptor invalid",
                ));
            }
            return Ok(());
        }
        if rc == 0 {
            return Err(io::Error::new(
                ErrorKind::TimedOut,
                "sidecar response timed out",
            ));
        }
        let e = io::Error::last_os_error();
        if e.kind() != ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

impl LlmProvider for FoundationLlm {
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let result: CompleteResult = self.send_request(
            "complete",
            CompleteParams {
                system: req.system.as_deref(),
                user: &req.user,
                schema: req.schema.as_ref(),
                temperature: req.temperature,
                max_tokens: req.max_tokens,
            },
        )?;
        match (result.json, result.text) {
            (Some(v), _) => Ok(LlmResponse::Json(v)),
            (None, Some(s)) if !s.is_empty() => Ok(LlmResponse::Text(s)),
            (None, Some(_)) => Err(LlmError::EmptyResponse),
            (None, None) => Err(LlmError::EmptyResponse),
        }
    }
}

impl Drop for FoundationLlm {
    fn drop(&mut self) {
        // `SidecarState`'s own `Drop` closes stdin and reaps the child;
        // we just need to take the slot here so it runs even if the
        // mutex is held by no one.
        if let Ok(mut slot) = self.state.lock() {
            slot.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_id_monotonic() {
        let llm = FoundationLlm::new(PathBuf::from("/nonexistent"));
        let ids: Vec<u64> = (0..5).map(|_| llm.next_id().expect("next_id")).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }
}
