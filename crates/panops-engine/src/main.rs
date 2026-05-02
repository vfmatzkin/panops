//! `panops-engine` — dev/CI driver for the panops engine. Not the product UX.
//! See https://github.com/vfmatzkin/panops for the desktop app.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use panops_core::asr::AsrProvider;
use panops_core::conformance::fakes::TranscriptFileFake;
use panops_core::diar::Diarizer;
use panops_core::exporter::NotesExporter;
use panops_core::merge::merge_speaker_turns;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::input::{MeetingMetadata, NotesInput};
use panops_core::notes::ir::Screenshot;
use panops_core::notes::pipeline::NotesGenerator;
use panops_portable::SherpaDiarizer;
use panops_portable::WhisperRsAsr;
use panops_portable::genai_llm::GenaiLlm;
use panops_portable::markdown_exporter::MarkdownExporter;
use panops_portable::model::{
    DEFAULT_MODEL_NAME, default_model_path, ensure_diar_models, ensure_model,
};

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,

    /// (Default mode, no subcommand) Path to a 16 kHz mono WAV.
    audio: Option<PathBuf>,

    #[arg(long)]
    model: Option<PathBuf>,

    #[arg(long)]
    language: Option<String>,

    #[arg(long)]
    no_diarize: bool,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Generate markdown meeting notes from an audio file.
    Notes {
        audio: PathBuf,
        #[arg(long)]
        screenshots: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = DialectArg::NotionEnhanced)]
        dialect: DialectArg,
        #[arg(long)]
        no_diarize: bool,
        #[arg(long, default_value = "auto")]
        llm_provider: String,
        #[arg(long)]
        llm_model: Option<String>,
        #[arg(long)]
        model: Option<PathBuf>,
        #[arg(long)]
        language: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DialectArg {
    NotionEnhanced,
    Basic,
}

impl From<DialectArg> for MarkdownDialect {
    fn from(d: DialectArg) -> Self {
        match d {
            DialectArg::NotionEnhanced => MarkdownDialect::NotionEnhanced,
            DialectArg::Basic => MarkdownDialect::Basic,
        }
    }
}

fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    let res = match cli.cmd {
        None => run_default(cli.audio, cli.model, cli.language, cli.no_diarize),
        Some(Cmd::Notes {
            audio,
            screenshots,
            out,
            dialect,
            no_diarize,
            llm_provider,
            llm_model,
            model,
            language,
        }) => run_notes(
            audio,
            screenshots,
            out,
            dialect.into(),
            no_diarize,
            llm_provider,
            llm_model,
            model,
            language,
        ),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(code)
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn run_default(
    audio: Option<PathBuf>,
    model: Option<PathBuf>,
    language: Option<String>,
    no_diarize: bool,
) -> Result<(), (u8, String)> {
    let audio = audio.ok_or((1, "audio path required".to_string()))?;
    let mut transcript = transcribe(&audio, model, language.as_deref())?;
    if !no_diarize {
        let turns = diarize(&audio)?;
        transcript.segments = merge_speaker_turns(transcript.segments, &turns);
        transcript.diarized = true;
    }
    let json =
        serde_json::to_string_pretty(&transcript).map_err(|e| (2, format!("serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_notes(
    audio: PathBuf,
    screenshots_dir: Option<PathBuf>,
    out: Option<PathBuf>,
    dialect: MarkdownDialect,
    no_diarize: bool,
    llm_provider: String,
    llm_model: Option<String>,
    model: Option<PathBuf>,
    language: Option<String>,
) -> Result<(), (u8, String)> {
    let mut transcript = transcribe(&audio, model, language.as_deref())?;
    if !no_diarize {
        let turns = diarize(&audio)?;
        transcript.segments = merge_speaker_turns(transcript.segments, &turns);
        transcript.diarized = true;
    }

    let llm = match llm_provider.as_str() {
        "auto" => match llm_model {
            Some(m) => GenaiLlm::new(m).map_err(|e| (3, e.to_string()))?,
            None => GenaiLlm::auto().map_err(|e| (3, e.to_string()))?,
        },
        "ollama" => {
            let model = llm_model.unwrap_or_else(|| "gemma3:4b".to_string());
            GenaiLlm::new(model).map_err(|e| (3, e.to_string()))?
        }
        other => {
            return Err((
                1,
                format!(
                    "--llm-provider {other:?} not supported. Use \"auto\" (detects from \
                     ANTHROPIC_API_KEY / OPENAI_API_KEY / OLLAMA_HOST) or \"ollama\" \
                     (defaults to model gemma3:4b on http://localhost:11434)."
                ),
            ));
        }
    };

    let screenshots = screenshots_dir
        .as_ref()
        .map(|d| collect_screenshots(d, transcript.audio_duration_ms))
        .transpose()?
        .unwrap_or_default();

    let started_at = chrono::Local::now().fixed_offset();
    let input = NotesInput {
        transcript: transcript.segments,
        screenshots,
        meeting_metadata: MeetingMetadata {
            started_at,
            duration_ms: transcript.audio_duration_ms,
            source_path: Some(audio.clone()),
            language_hint: language,
        },
    };

    let generator = NotesGenerator { llm: &llm, dialect };
    let notes = generator.generate(input).map_err(|e| (2, e.to_string()))?;

    let out_dir = out.unwrap_or_else(|| {
        let stem = audio
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "notes".to_string());
        PathBuf::from(format!("./{stem}-notes"))
    });
    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir).map_err(|e| (3, e.to_string()))?;
    }
    let art = MarkdownExporter
        .export(&notes, &out_dir)
        .map_err(|e| (2, e.to_string()))?;
    tracing::info!(file = ?art.primary_file, assets = art.assets.len(), "wrote notes");
    Ok(())
}

fn collect_screenshots(
    dir: &std::path::Path,
    duration_ms: u64,
) -> Result<Vec<Screenshot>, (u8, String)> {
    if !dir.exists() {
        return Err((1, format!("screenshots dir not found: {dir:?}")));
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| (3, format!("read_dir {dir:?}: {e}")))?
        .filter_map(|r| r.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    entries.sort();
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let n = entries.len() as u64;
    let step = duration_ms.checked_div(n).unwrap_or(0);
    Ok(entries
        .into_iter()
        .enumerate()
        .map(|(i, path)| Screenshot {
            ms_since_start: (i as u64) * step,
            path,
            caption: None,
        })
        .collect())
}

fn transcribe(
    audio: &std::path::Path,
    model: Option<PathBuf>,
    language: Option<&str>,
) -> Result<panops_core::Transcript, (u8, String)> {
    if !audio.exists() {
        return Err((1, format!("audio file not found: {audio:?}")));
    }
    // When PANOPS_FAKE_ASR=1, use the sidecar-file fake instead of downloading
    // and loading the real Whisper model. Intended for integration tests.
    if std::env::var("PANOPS_FAKE_ASR").ok().as_deref() == Some("1") {
        return TranscriptFileFake
            .transcribe_full(audio, language)
            .map_err(|e| (2, e.to_string()));
    }
    let model_path = match model {
        Some(p) => p,
        None => default_model_path().map_err(|e| (3, e.to_string()))?,
    };
    let model_path =
        ensure_model(DEFAULT_MODEL_NAME, &model_path).map_err(|e| (3, e.to_string()))?;
    let asr = WhisperRsAsr::new(model_path).map_err(|e| (3, e.to_string()))?;
    asr.transcribe_full(audio, language)
        .map_err(|e| (2, e.to_string()))
}

fn diarize(audio: &std::path::Path) -> Result<Vec<panops_core::diar::SpeakerTurn>, (u8, String)> {
    let (seg, emb) = ensure_diar_models().map_err(|e| (3, e.to_string()))?;
    let diar = SherpaDiarizer::new(seg, emb).map_err(|e| (3, e.to_string()))?;
    diar.diarize(audio).map_err(|e| (2, e.to_string()))
}
