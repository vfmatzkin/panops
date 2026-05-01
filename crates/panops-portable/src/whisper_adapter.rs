use std::path::{Path, PathBuf};

use panops_core::Segment;
use panops_core::Transcript;
use panops_core::asr::{AsrError, AsrProvider};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
    convert_integer_to_float_audio, convert_stereo_to_mono_audio,
};

pub struct WhisperRsAsr {
    model_path: PathBuf,
    ctx: WhisperContext,
}

impl WhisperRsAsr {
    pub fn new(model_path: PathBuf) -> Result<Self, AsrError> {
        if !model_path.is_file() {
            return Err(AsrError::Model(format!(
                "expected model path to be a file: {model_path:?}"
            )));
        }
        let path_str = model_path
            .to_str()
            .ok_or_else(|| AsrError::Model("non-UTF-8 model path".to_string()))?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .map_err(|e| AsrError::Model(e.to_string()))?;
        Ok(Self { model_path, ctx })
    }

    fn model_name(&self) -> String {
        self.model_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    }
}

impl AsrProvider for WhisperRsAsr {
    fn transcribe_full(
        &self,
        audio_path: &Path,
        language_hint: Option<&str>,
    ) -> Result<Transcript, AsrError> {
        if !audio_path.exists() {
            return Err(AsrError::AudioNotFound(audio_path.to_path_buf()));
        }
        let reader = hound::WavReader::open(audio_path)
            .map_err(|e| AsrError::InvalidAudio(e.to_string()))?;
        let spec = reader.spec();
        if spec.sample_rate != 16_000 {
            return Err(AsrError::InvalidAudio(format!(
                "expected 16 kHz, got {} Hz",
                spec.sample_rate
            )));
        }
        let samples_i16: Vec<i16> = reader
            .into_samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AsrError::InvalidAudio(e.to_string()))?;

        let mut audio_f32 = vec![0.0_f32; samples_i16.len()];
        convert_integer_to_float_audio(&samples_i16, &mut audio_f32)
            .map_err(|e| AsrError::InvalidAudio(e.to_string()))?;

        let audio = if spec.channels == 2 {
            let mono_len = audio_f32.len() / 2;
            let mut mono = vec![0.0_f32; mono_len];
            convert_stereo_to_mono_audio(&audio_f32, &mut mono)
                .map_err(|e| AsrError::InvalidAudio(e.to_string()))?;
            mono
        } else if spec.channels == 1 {
            audio_f32
        } else {
            return Err(AsrError::InvalidAudio(format!(
                "expected 1 or 2 channels, got {}",
                spec.channels
            )));
        };

        let audio_duration_ms = (audio.len() as f64 * 1000.0 / 16_000.0) as u64;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(language_hint);
        params.set_print_progress(false);
        params.set_print_special(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        let parallelism = std::thread::available_parallelism()
            .map(std::num::NonZero::get)
            .unwrap_or(1);
        let n_threads = std::cmp::min(parallelism, 8) as i32;
        params.set_n_threads(n_threads);

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| AsrError::Transcription(e.to_string()))?;
        state
            .full(params, &audio)
            .map_err(|e| AsrError::Transcription(e.to_string()))?;

        // full_lang_id_from_state returns c_int directly (not Result) in 0.16
        let detected_lang = match language_hint {
            Some(hint) => Some(hint.to_string()),
            None => {
                let lang_id = state.full_lang_id_from_state();
                whisper_rs::get_lang_str(lang_id).map(str::to_string)
            }
        };

        let n_segments = state.full_n_segments();
        let mut segments = Vec::with_capacity(n_segments as usize);
        for i in 0..n_segments {
            let seg = state
                .get_segment(i)
                .ok_or_else(|| AsrError::Transcription(format!("segment {i} missing")))?;

            let text = seg
                .to_str_lossy()
                .map_err(|e| AsrError::Transcription(e.to_string()))?
                .trim()
                .to_string();
            let t0 = seg.start_timestamp();
            let t1 = seg.end_timestamp();

            let n_tok = seg.n_tokens();
            let mut prob_sum = 0.0_f32;
            for j in 0..n_tok {
                if let Some(tok) = seg.get_token(j) {
                    prob_sum += tok.token_probability();
                }
            }
            let confidence = if n_tok > 0 {
                prob_sum / n_tok as f32
            } else {
                0.0
            };

            segments.push(Segment {
                start_ms: (t0.max(0) as u64 * 10).min(audio_duration_ms),
                end_ms: (t1.max(0) as u64 * 10).min(audio_duration_ms),
                text,
                language_detected: detected_lang.clone(),
                confidence,
                is_partial: false,
            });
        }

        Ok(Transcript {
            schema_version: Transcript::SCHEMA_VERSION,
            model: self.model_name(),
            audio_path: audio_path.to_path_buf(),
            audio_duration_ms,
            segments,
        })
    }
}
