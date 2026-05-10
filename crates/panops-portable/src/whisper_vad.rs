//! Real `Vad` adapter wrapping `whisper_rs::WhisperVadContext`.
//! Holds a single `WhisperVadContext` behind a mutex (Whisper's
//! VAD context methods take `&mut self`). Single-user local-first;
//! mutex serializes calls, no contention concern at v0.1 scale.
//!
//! Model: `ggml-silero-v6.2.0.bin` (whisper.cpp's bundled Silero
//! VAD), downloaded by `panops_portable::model::ensure_vad_model`.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use panops_core::vad::{SpeechRegion, Vad, VadError};
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

pub struct WhisperVad {
    ctx: Mutex<WhisperVadContext>,
}

fn lock<'a>(
    m: &'a Mutex<WhisperVadContext>,
) -> Result<MutexGuard<'a, WhisperVadContext>, VadError> {
    m.lock()
        .map_err(|e| VadError::Model(format!("vad mutex poisoned: {e}")))
}

impl WhisperVad {
    /// Open the VAD model at `model_path`. Returns an error if the
    /// path doesn't exist or is not a valid GGML VAD model file.
    pub fn new(model_path: &Path) -> Result<Self, VadError> {
        if !model_path.is_file() {
            return Err(VadError::Model(format!(
                "expected vad model path to be a file: {model_path:?}"
            )));
        }
        let path_str = model_path
            .to_str()
            .ok_or_else(|| VadError::Model("non-UTF-8 vad model path".to_string()))?;
        let ctx = WhisperVadContext::new(path_str, WhisperVadContextParams::default())
            .map_err(|e| VadError::Model(e.to_string()))?;
        Ok(Self {
            ctx: Mutex::new(ctx),
        })
    }
}

impl Vad for WhisperVad {
    fn detect_speech(
        &self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<SpeechRegion>, VadError> {
        if sample_rate != 16_000 {
            return Err(VadError::InvalidAudio(format!(
                "expected 16 kHz, got {sample_rate} Hz"
            )));
        }
        let mut ctx = lock(&self.ctx)?;
        let segments = ctx
            .segments_from_samples(WhisperVadParams::default(), samples)
            .map_err(|e| VadError::Model(e.to_string()))?;

        let mut regions: Vec<SpeechRegion> = Vec::new();
        for seg in segments {
            // `seg.start` and `seg.end` are f32 centiseconds;
            // multiply by 10 to get milliseconds.
            let start_ms = (seg.start * 10.0).max(0.0) as u64;
            let end_ms = (seg.end * 10.0).max(0.0) as u64;
            if end_ms > start_ms {
                regions.push(SpeechRegion { start_ms, end_ms });
            }
        }
        regions.sort_by_key(|r| r.start_ms);
        Ok(regions)
    }
}
