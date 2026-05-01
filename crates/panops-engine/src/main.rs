//! `panops-engine` — dev/CI driver for the panops engine. Not the product UX.
//! See https://github.com/vfmatzkin/panops for the desktop app.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use panops_core::asr::AsrProvider;
use panops_core::diar::Diarizer;
use panops_core::merge::merge_speaker_turns;
use panops_portable::SherpaDiarizer;
use panops_portable::WhisperRsAsr;
use panops_portable::model::{
    DEFAULT_MODEL_NAME, default_model_path, ensure_diar_models, ensure_model,
};

/// panops engine. Dev/CI tool. Not for end users; see Panops.app.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to a 16 kHz mono WAV file.
    audio: PathBuf,

    /// Override the default Whisper model path. Defaults to the cross-platform
    /// data dir (~/Library/Application Support/panops/models on macOS).
    /// Honors $PANOPS_MODEL.
    #[arg(long)]
    model: Option<PathBuf>,

    /// ISO 639-1 language hint (e.g. "en", "es"). Default: auto-detect.
    #[arg(long)]
    language: Option<String>,

    /// Skip the diarization pass. Faster; segments have speaker_id = null.
    #[arg(long)]
    no_diarize: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(code)
        }
    }
}

fn run(args: Args) -> Result<(), (u8, String)> {
    if !args.audio.exists() {
        return Err((1, format!("audio file not found: {:?}", args.audio)));
    }

    // ASR
    let model_path = match args.model {
        Some(p) => p,
        None => default_model_path().map_err(|e| (3, e.to_string()))?,
    };
    let model_path =
        ensure_model(DEFAULT_MODEL_NAME, &model_path).map_err(|e| (3, e.to_string()))?;
    let asr = WhisperRsAsr::new(model_path).map_err(|e| (3, e.to_string()))?;
    let mut transcript = asr
        .transcribe_full(&args.audio, args.language.as_deref())
        .map_err(|e| (2, e.to_string()))?;

    // Diarization (default on; --no-diarize opts out)
    if !args.no_diarize {
        let (seg, emb) = ensure_diar_models().map_err(|e| (3, e.to_string()))?;
        let diar = SherpaDiarizer::new(seg, emb).map_err(|e| (3, e.to_string()))?;
        let turns = diar.diarize(&args.audio).map_err(|e| (2, e.to_string()))?;
        transcript.segments = merge_speaker_turns(transcript.segments, &turns);
        transcript.diarized = true;
    }

    let json =
        serde_json::to_string_pretty(&transcript).map_err(|e| (2, format!("serialize: {e}")))?;
    println!("{json}");
    Ok(())
}
