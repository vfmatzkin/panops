use std::path::{Path, PathBuf};

use panops_core::diar::{DiarError, Diarizer, SpeakerTurn};
use sherpa_rs::diarize::{Diarize, DiarizeConfig};
use sherpa_rs::read_audio_file;

pub struct SherpaDiarizer {
    seg_path: PathBuf,
    emb_path: PathBuf,
}

impl SherpaDiarizer {
    pub fn new(seg_path: PathBuf, emb_path: PathBuf) -> Result<Self, DiarError> {
        if !seg_path.is_file() {
            return Err(DiarError::Model(format!(
                "segmentation model not found: {seg_path:?}"
            )));
        }
        if !emb_path.is_file() {
            return Err(DiarError::Model(format!(
                "embedding model not found: {emb_path:?}"
            )));
        }
        Ok(Self { seg_path, emb_path })
    }
}

impl Diarizer for SherpaDiarizer {
    fn diarize(&self, audio_path: &Path) -> Result<Vec<SpeakerTurn>, DiarError> {
        if !audio_path.is_file() {
            return Err(DiarError::AudioNotFound(audio_path.to_path_buf()));
        }

        let (samples, sample_rate) = read_audio_file(
            audio_path
                .to_str()
                .ok_or_else(|| DiarError::InvalidAudio("non-UTF-8 audio path".to_string()))?,
        )
        .map_err(|e| DiarError::InvalidAudio(format!("read audio: {e}")))?;
        if sample_rate != 16_000 {
            return Err(DiarError::InvalidAudio(format!(
                "expected 16 kHz, got {sample_rate} Hz"
            )));
        }

        let config = DiarizeConfig {
            num_clusters: None,
            ..Default::default()
        };
        let mut sd = Diarize::new(
            self.seg_path
                .to_str()
                .ok_or_else(|| DiarError::Model("non-UTF-8 seg path".to_string()))?,
            self.emb_path
                .to_str()
                .ok_or_else(|| DiarError::Model("non-UTF-8 emb path".to_string()))?,
            config,
        )
        .map_err(|e| DiarError::Model(format!("init Diarize: {e}")))?;

        let segments = sd
            .compute(samples, None)
            .map_err(|e| DiarError::Diarization(format!("compute: {e}")))?;

        let mut turns: Vec<SpeakerTurn> = segments
            .into_iter()
            .map(|s| SpeakerTurn {
                start_ms: (s.start * 1000.0) as u64,
                end_ms: (s.end * 1000.0) as u64,
                speaker_id: s.speaker as u32,
            })
            .collect();
        turns.sort_by_key(|t| t.start_ms);
        // sherpa rarely emits overlapping turns, but the conformance
        // suite asserts non-overlapping ordering. Clamp defensively.
        for i in 1..turns.len() {
            let prev_end = turns[i - 1].end_ms;
            if turns[i].start_ms < prev_end {
                turns[i].start_ms = prev_end;
            }
            if turns[i].end_ms < turns[i].start_ms {
                turns[i].end_ms = turns[i].start_ms;
            }
        }
        Ok(turns)
    }
}
