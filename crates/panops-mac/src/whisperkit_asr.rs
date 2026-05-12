//! WhisperKit ASR sidecar adapter. Spawns `panops-asr-mac` as a
//! child process and communicates via newline-delimited JSON-RPC
//! over stdio. Lazy spawn on first transcribe; reused across calls
//! to amortize the ~3-5s model load.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use panops_core::Transcript;
use panops_core::asr::{AsrError, AsrProvider};
use serde::{Deserialize, Deserializer, Serialize};

/// Reject sidecar response lines larger than 16 MiB. A 60-second
/// transcript is tens of KB; this cap is purely a safety bound
/// against a wedged sidecar emitting unbounded output.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

pub struct WhisperKitAsr {
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

#[derive(Serialize)]
struct TranscribeParams<'a> {
    audio: &'a str,
    sample_rate: u32,
    language_hint: Option<&'a str>,
}

#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: [TranscribeParams<'a>; 1],
}

#[derive(Deserialize)]
struct RpcResponse {
    #[serde(rename = "jsonrpc", deserialize_with = "deserialize_jsonrpc_version")]
    _jsonrpc: (),
    /// `Option` so a sidecar parse-error response with `id: null`
    /// (JSON-RPC 2.0 §4) decodes instead of failing — we still
    /// surface it as an error because we can't pair it to a request.
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    result: Option<Transcript>,
    #[serde(default)]
    error: Option<RpcError>,
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

#[derive(Deserialize, Debug)]
struct RpcError {
    code: i32,
    message: String,
}

impl WhisperKitAsr {
    pub fn new(binary: PathBuf) -> Self {
        Self {
            binary,
            state: Mutex::new(None),
            next_id: Mutex::new(1),
        }
    }

    fn ensure_spawned(&self, slot: &mut Option<SidecarState>) -> Result<(), AsrError> {
        if slot.is_some() {
            return Ok(());
        }
        tracing::info!(binary = %self.binary.display(), "spawning panops-asr-mac sidecar");
        let mut child = Command::new(&self.binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Sidecar's stderr passes through to engine's stderr so
            // WhisperKit load progress / errors land in Console.app.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| AsrError::Model(format!("sidecar spawn: {e}")))?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        *slot = Some(SidecarState {
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
        });
        Ok(())
    }

    fn next_id(&self) -> Result<u64, AsrError> {
        // Map a poisoned next_id mutex to a typed error instead of
        // panicking on a future ASR call. The lock is held only during
        // an integer increment, so poison is exceedingly unlikely —
        // but per panops's "no panics in production" discipline, surface
        // it cleanly.
        let mut id = self
            .next_id
            .lock()
            .map_err(|e| AsrError::Transcription(format!("next_id mutex poisoned: {e}")))?;
        let v = *id;
        *id += 1;
        Ok(v)
    }
}

impl AsrProvider for WhisperKitAsr {
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        // The sidecar reads audio from disk (avoids base64'ing audio
        // over JSON-RPC). Write a temp WAV, hand the path over, drop
        // the temp file when done.
        let wav = write_temp_wav(samples, sample_rate)?;
        self.send_request(wav.path(), sample_rate, language_hint)
    }
}

impl WhisperKitAsr {
    fn send_request(
        &self,
        audio_path: &Path,
        sample_rate: u32,
        language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        let mut slot = self
            .state
            .lock()
            .map_err(|e| AsrError::Transcription(format!("sidecar state mutex poisoned: {e}")))?;
        self.ensure_spawned(&mut slot)?;

        let id = self.next_id()?;
        let req = RpcRequest {
            jsonrpc: "2.0",
            id,
            method: "asr.transcribe",
            params: [TranscribeParams {
                audio: audio_path
                    .to_str()
                    .ok_or_else(|| AsrError::InvalidAudio("temp WAV path is not utf-8".into()))?,
                sample_rate,
                language_hint,
            }],
        };
        let body = serde_json::to_vec(&req)
            .map_err(|e| AsrError::Transcription(format!("encode: {e}")))?;

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
            return Err(AsrError::Transcription(format!("stdio write: {e}")));
        }

        // Bound the response read against a wedged sidecar emitting an
        // unbounded line. `take(MAX_RESPONSE_BYTES)` + `read_until(b'\n')`
        // gives us a strict cap; if we read the cap and the last byte
        // isn't a newline, the line was truncated and framing is lost.
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
                return Err(AsrError::Transcription(format!("stdio read: {e}")));
            }
        };
        if n == 0 {
            // Sidecar exited; drop the state so the next call respawns.
            *slot = None;
            return Err(AsrError::Transcription(
                "sidecar exited before responding".into(),
            ));
        }
        if buf.last() != Some(&b'\n') {
            // No trailing newline means either (a) the sidecar died
            // mid-line (EOF before completing the response) or (b) the
            // line hit the `MAX_RESPONSE_BYTES` cap. Distinguish so the
            // error message points at the right failure mode during
            // debugging — wedged-sidecar vs framing-corrupt look very
            // different in practice.
            *slot = None;
            let msg = if (n as u64) >= MAX_RESPONSE_BYTES {
                format!("sidecar response exceeded {MAX_RESPONSE_BYTES} bytes without newline")
            } else {
                format!("sidecar response truncated at {n} bytes (EOF mid-line)")
            };
            return Err(AsrError::Transcription(msg));
        }
        let line = std::str::from_utf8(&buf)
            .map_err(|e| AsrError::Transcription(format!("response not utf-8: {e}")))?;
        let resp: RpcResponse = serde_json::from_str(line.trim_end()).map_err(|e| {
            *slot = None;
            AsrError::Transcription(format!("decode: {e}"))
        })?;
        // Validate JSON-RPC id pairing. A `None` id is a parse-error
        // response per JSON-RPC 2.0 §4; a mismatched id means we lost
        // framing. Either way the stream is corrupt and the sidecar
        // must be respawned.
        match resp.id {
            Some(rid) if rid == id => {}
            Some(rid) => {
                *slot = None;
                return Err(AsrError::Transcription(format!(
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
                return Err(AsrError::Transcription(format!(
                    "sidecar returned null id{detail}"
                )));
            }
        }
        if let Some(err) = resp.error {
            // Log full sidecar error detail (model paths, CoreML state)
            // locally; surface an opaque, code-only message over the
            // wire so the engine's IPC `Internal` error doesn't echo
            // sidecar internals to clients.
            tracing::error!(
                code = err.code,
                message = %err.message,
                "panops-asr-mac sidecar returned error"
            );
            return Err(AsrError::Transcription(format!(
                "sidecar error (code {})",
                err.code
            )));
        }
        resp.result
            .ok_or_else(|| AsrError::Transcription("response missing result".into()))
    }
}

impl Drop for WhisperKitAsr {
    fn drop(&mut self) {
        // `SidecarState`'s own `Drop` closes stdin and reaps the child;
        // we just need to take the slot here so it runs even if the
        // mutex is held by no one. Poisoned mutex still gives access
        // via `into_inner`-equivalent semantics through the guard.
        if let Ok(mut slot) = self.state.lock() {
            slot.take();
        }
    }
}

fn write_temp_wav(samples: &[f32], sample_rate: u32) -> Result<tempfile::NamedTempFile, AsrError> {
    let tmp = tempfile::Builder::new()
        .prefix("panops-asr-")
        .suffix(".wav")
        .tempfile()
        .map_err(AsrError::Io)?;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(tmp.path(), spec)
        .map_err(|e| AsrError::InvalidAudio(format!("create wav: {e}")))?;
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(v)
            .map_err(|e| AsrError::InvalidAudio(format!("write sample: {e}")))?;
    }
    writer
        .finalize()
        .map_err(|e| AsrError::InvalidAudio(format!("finalize wav: {e}")))?;
    Ok(tmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_wav_roundtrips() {
        let samples = vec![0.0_f32, 0.5, -0.5, 1.0, -1.0];
        let tmp = write_temp_wav(&samples, 16_000).expect("temp wav");
        let reader = hound::WavReader::open(tmp.path()).expect("read wav");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        let read_samples: Vec<i16> = reader
            .into_samples::<i16>()
            .map(|r| r.expect("sample"))
            .collect();
        assert_eq!(read_samples.len(), 5);
    }

    #[test]
    fn next_id_monotonic() {
        let asr = WhisperKitAsr::new(PathBuf::from("/nonexistent"));
        let ids: Vec<u64> = (0..5).map(|_| asr.next_id().expect("next_id")).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }
}
