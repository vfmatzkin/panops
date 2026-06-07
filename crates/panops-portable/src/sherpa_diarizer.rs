use std::collections::HashMap;
use std::path::{Path, PathBuf};

use panops_core::diar::{DiarError, Diarizer, SpeakerTurn};
use sherpa_rs::diarize::{Diarize, DiarizeConfig};
use sherpa_rs::read_audio_file;

/// A speaker whose total speech is below this share of all speech is treated as
/// a spurious micro-cluster and folded into the nearest real speaker. sherpa-rs
/// cannot resolve the true speaker count on its own: auto mode over-segments a
/// real 2-speaker meeting into 14-33 clusters, and the fixed `num_clusters`
/// path pads it to 4 (two real speakers plus two tiny phantom clusters). This
/// threshold collapses those phantoms. v0.1 stopgap; the proper fix is a
/// pyannote-grade clusterer (#107).
const MINOR_SPEAKER_MAX_SHARE: f64 = 0.05;

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

        // Accept any CoreAudio-decodable container (WAV, MOV, MP4, M4A, …):
        // a ready 16 kHz WAV is read directly, anything else is transcoded to
        // a temp 16 kHz WAV first — same media path the ASR pipeline uses, so
        // diarization works on the same inputs transcription does.
        // Map variants explicitly so a filesystem path only ever lands in the
        // typed `AudioNotFound` (never expanded into a free-form error string).
        let wav = crate::audio::ensure_wav16k(audio_path).map_err(|e| match e {
            panops_core::asr::AsrError::AudioNotFound(p) => DiarError::AudioNotFound(p),
            other => DiarError::InvalidAudio(other.to_string()),
        })?;

        let (samples, sample_rate) = read_audio_file(
            wav.path()
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

        // Fold spurious tiny speaker clusters into the nearest real speaker so a
        // real 2-speaker meeting stops being reported as 4. No-op when every
        // speaker carries a meaningful share of speech.
        collapse_minor_speakers(&mut turns);

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

/// Collapse spurious tiny speaker clusters in a sorted turn list.
///
/// A speaker whose total speech is below [`MINOR_SPEAKER_MAX_SHARE`] of all
/// speech is "minor". Each minor turn is relabeled to the temporally nearest
/// major speaker (the closest preceding major turn, else the closest following
/// one); newly-adjacent same-speaker turns are merged and speaker ids are
/// renumbered contiguously from 0. No-op when no speaker is minor, or when no
/// major speaker exists to fold into.
///
/// Expects `turns` sorted by `start_ms`; preserves that ordering.
fn collapse_minor_speakers(turns: &mut Vec<SpeakerTurn>) {
    if turns.is_empty() {
        return;
    }

    let mut totals: HashMap<u32, u64> = HashMap::new();
    let mut total_all: u64 = 0;
    for t in turns.iter() {
        let dur = t.end_ms.saturating_sub(t.start_ms);
        *totals.entry(t.speaker_id).or_default() += dur;
        total_all += dur;
    }
    if total_all == 0 {
        return;
    }

    let threshold = total_all as f64 * MINOR_SPEAKER_MAX_SHARE;
    let is_minor = |spk: u32| (totals[&spk] as f64) < threshold;

    let has_minor = totals.keys().any(|&spk| is_minor(spk));
    let has_major = totals.keys().any(|&spk| !is_minor(spk));
    if !has_minor || !has_major {
        return;
    }

    // Relabel each minor turn to the nearest major speaker. Processing left to
    // right and relabeling in place means a backward scan naturally picks up
    // the immediately-preceding turn's (possibly just-corrected) major id.
    for i in 0..turns.len() {
        if !is_minor(turns[i].speaker_id) {
            continue;
        }
        let target = (0..i)
            .rev()
            .map(|j| turns[j].speaker_id)
            .find(|&spk| !is_minor(spk))
            .or_else(|| {
                ((i + 1)..turns.len())
                    .map(|j| turns[j].speaker_id)
                    .find(|&spk| !is_minor(spk))
            });
        if let Some(spk) = target {
            turns[i].speaker_id = spk;
        }
    }

    // Merge runs of the same speaker created by relabeling.
    let mut merged: Vec<SpeakerTurn> = Vec::with_capacity(turns.len());
    for t in turns.iter() {
        match merged.last_mut() {
            Some(last) if last.speaker_id == t.speaker_id => {
                last.end_ms = last.end_ms.max(t.end_ms);
            }
            _ => merged.push(*t),
        }
    }
    *turns = merged;

    // Renumber speaker ids contiguously from 0 in order of first appearance.
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut next: u32 = 0;
    for t in turns.iter_mut() {
        let id = *remap.entry(t.speaker_id).or_insert_with(|| {
            let v = next;
            next += 1;
            v
        });
        t.speaker_id = id;
    }
}
