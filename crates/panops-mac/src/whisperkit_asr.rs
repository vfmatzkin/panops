//! WhisperKit ASR sidecar adapter. Spawns `panops-asr-mac` as a
//! child process and communicates via newline-delimited JSON-RPC
//! over stdio. Lazy spawn on first transcribe; reused across calls
//! to amortize the ~3-5s model load.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use panops_core::Transcript;
use panops_core::asr::{AsrError, AsrProvider};
use serde::{Deserialize, Serialize};

pub struct WhisperKitAsr {
    binary: PathBuf,
    state: Mutex<Option<SidecarState>>,
    next_id: Mutex<u64>,
}

struct SidecarState {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
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
    #[serde(default)]
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: u64,
    #[serde(default)]
    result: Option<Transcript>,
    #[serde(default)]
    error: Option<RpcError>,
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
            child,
            stdin,
            stdout: BufReader::new(stdout),
        });
        Ok(())
    }

    fn next_id(&self) -> u64 {
        let mut id = self.next_id.lock().expect("next_id mutex poisoned");
        let v = *id;
        *id += 1;
        v
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
        let mut slot = self.state.lock().expect("sidecar state mutex poisoned");
        self.ensure_spawned(&mut slot)?;
        let state = slot.as_mut().expect("just spawned");

        let id = self.next_id();
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
        state
            .stdin
            .write_all(&body)
            .and_then(|()| state.stdin.write_all(b"\n"))
            .and_then(|()| state.stdin.flush())
            .map_err(|e| AsrError::Transcription(format!("stdio write: {e}")))?;

        let mut line = String::new();
        let n = state
            .stdout
            .read_line(&mut line)
            .map_err(|e| AsrError::Transcription(format!("stdio read: {e}")))?;
        if n == 0 {
            // Sidecar exited; drop the state so the next call respawns.
            *slot = None;
            return Err(AsrError::Transcription(
                "sidecar exited before responding".into(),
            ));
        }
        let resp: RpcResponse = serde_json::from_str(line.trim_end())
            .map_err(|e| AsrError::Transcription(format!("decode: {e}")))?;
        if let Some(err) = resp.error {
            return Err(AsrError::Transcription(format!(
                "sidecar error {}: {}",
                err.code, err.message
            )));
        }
        resp.result
            .ok_or_else(|| AsrError::Transcription("response missing result".into()))
    }
}

impl Drop for WhisperKitAsr {
    fn drop(&mut self) {
        let mut slot = self
            .state
            .lock()
            .expect("sidecar state mutex poisoned on drop");
        if let Some(state) = slot.take() {
            // Take ownership of `state` so `stdin` is dropped here,
            // closing the sidecar's stdin. The sidecar's `while let
            // Some(line) = readLine()` loop sees EOF and exits.
            let SidecarState {
                mut child,
                stdin,
                stdout,
            } = state;
            drop(stdin);
            drop(stdout);
            // Fallback in case the sidecar is wedged after stdin close.
            let _ = child.kill();
            let _ = child.wait();
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
        let ids: Vec<u64> = (0..5).map(|_| asr.next_id()).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }
}
