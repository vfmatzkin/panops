//! `panops-engine` — dev/CI driver for the panops engine. Not the product UX.
//! See https://github.com/vfmatzkin/panops for the desktop app.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use panops_core::asr::AsrProvider;
use panops_portable::WhisperRsAsr;
use panops_portable::model::{DEFAULT_MODEL, default_model_path, ensure_model};

/// panops engine. Dev/CI tool. Not for end users; see Panops.app.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to a 16 kHz mono WAV file.
    audio: PathBuf,

    /// Override the default model path. Defaults to the cross-platform
    /// data dir (~/Library/Application Support/panops/models on macOS).
    /// Honors $PANOPS_MODEL.
    #[arg(long)]
    model: Option<PathBuf>,

    /// ISO 639-1 language hint (e.g. "en", "es"). Default: auto-detect.
    #[arg(long)]
    language: Option<String>,
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

    let model_path = match args.model {
        Some(p) => p,
        None => default_model_path().map_err(|e| (3, e.to_string()))?,
    };
    let model_path = ensure_model(DEFAULT_MODEL, &model_path).map_err(|e| (3, e.to_string()))?;

    let asr = WhisperRsAsr::new(model_path).map_err(|e| (3, e.to_string()))?;
    let transcript = asr
        .transcribe_full(&args.audio, args.language.as_deref())
        .map_err(|e| (2, e.to_string()))?;

    let json =
        serde_json::to_string_pretty(&transcript).map_err(|e| (2, format!("serialize: {e}")))?;
    println!("{json}");
    Ok(())
}
