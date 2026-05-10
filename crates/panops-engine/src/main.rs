//! `panops-engine` — dev/CI driver for the panops engine. Not the product UX.
//! See https://github.com/vfmatzkin/panops for the desktop app.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use panops_core::asr::AsrProvider;
use panops_core::diar::Diarizer;
use panops_core::exporter::NotesExporter;
use panops_core::merge::merge_speaker_turns;
use panops_core::notes::dialect::MarkdownDialect;
use panops_core::notes::input::{MeetingMetadata, NotesInput};
use panops_core::notes::ir::Screenshot;
use panops_core::notes::pipeline::NotesGenerator;
use panops_core::storage::{MeetingDraft, NoteDraft, Storage};
use panops_core::vad::Vad;
use panops_portable::SherpaDiarizer;
use panops_portable::WhisperRsAsr;
use panops_portable::genai_llm::GenaiLlm;
use panops_portable::markdown_exporter::MarkdownExporter;
use panops_portable::model::{
    DEFAULT_MODEL_NAME, default_model_path, ensure_diar_models, ensure_model,
};
use panops_portable::rusqlite_storage::RusqliteStorage;

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

    /// Override the data directory. Defaults to
    /// `~/Library/Application Support/panops/`. The `panops.db`
    /// registry and `meetings/<uuid>/` subdirs live here. No env
    /// var equivalent — per AGENTS.md "no env vars for user config".
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,
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
    /// Run the IPC server (JSON-RPC + WebSocket over a Unix domain socket).
    Serve {
        /// Override the socket path. Defaults to
        /// `~/Library/Application Support/panops/engine.sock`.
        #[arg(long)]
        socket: Option<PathBuf>,
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
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "panops-engine starting"
    );
    let cli = Cli::parse();
    let data_dir = default_or_resolved_data_dir(&cli.data_dir);
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
            data_dir.clone(),
        ),
        Some(Cmd::Serve { socket }) => panops_engine::server::run_serve(socket, data_dir),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(code)
        }
    }
}

/// Default data directory for the CLI. Mirrors `server::socket::default_socket_path`'s
/// HOME handling: requires an absolute, non-empty HOME; falls back to
/// `/tmp/panops` only if HOME is missing or relative (CI / unusual envs).
/// Tests pass `--data-dir` explicitly so this fallback is for shell use.
fn default_data_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && !p.as_os_str().is_empty())
        .map_or_else(
            || PathBuf::from("/tmp/panops"),
            |home| home.join("Library/Application Support/panops"),
        )
}

fn default_or_resolved_data_dir(opt: &Option<PathBuf>) -> PathBuf {
    opt.clone().unwrap_or_else(default_data_dir)
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    // Default `info` so model-download progress and "wrote notes" surface
    // without requiring RUST_LOG; runs can take minutes and silent waits get
    // filed as bugs. Third-party HTTP crates are pinned to `warn` to prevent
    // RUST_LOG=trace from leaking API keys via hyper/reqwest/h2 request
    // logging — genai sends Anthropic/OpenAI Authorization headers there.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info")
            .add_directive("hyper=warn".parse().expect("static directive"))
            .add_directive("reqwest=warn".parse().expect("static directive"))
            .add_directive("h2=warn".parse().expect("static directive"))
            .add_directive("rustls=warn".parse().expect("static directive"))
    });
    // try_init: in tests cargo may already have wired a subscriber; main()
    // runs once per process so this can't double-init in production.
    // with_ansi(false): stderr is often piped (CI capture, `2>err`); ANSI
    // escapes break grep/awk and the cli_logging.rs assertions.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
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
    data_dir: PathBuf,
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
    let language_for_meeting = language.clone();
    let audio_duration_ms = transcript.audio_duration_ms;
    let input = NotesInput {
        transcript: transcript.segments,
        screenshots,
        meeting_metadata: MeetingMetadata {
            started_at,
            duration_ms: audio_duration_ms,
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

    // Slice 06: register the meeting + note in the registry. The CLI
    // preserves its existing on-disk layout (notes alongside audio,
    // not under `<data_dir>/meetings/<uuid>/`) — only the registry
    // entry is new. Failure is non-fatal: the notes file is already
    // on disk; we surface the storage error as a warning + non-zero
    // exit so CI catches it but the user still has the artifact.
    let dir_path = out_dir
        .canonicalize()
        .unwrap_or(out_dir.clone())
        .to_string_lossy()
        .into_owned();
    let title = audio
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();
    let dialect_str = dialect.as_str();
    // Open `panops.db` once for this CLI invocation. The handle is
    // dropped at the end of `run_notes`. SQLite serialises concurrent
    // writers file-wide, so a long-running `panops-engine serve`
    // holding its own handle won't conflict — our brief insert just
    // queues. If usage ever grows beyond single-user, the handle is
    // a `&dyn Storage` arg to `register_meeting_in_registry` so a
    // future shared-handle pattern is a one-line caller change.
    std::fs::create_dir_all(&data_dir).map_err(|e| (3, format!("create data dir: {e}")))?;
    let storage = RusqliteStorage::new(&data_dir.join("panops.db"))
        .map_err(|e| (3, format!("open registry: {e}")))?;
    register_meeting_in_registry(
        &storage,
        RegisterInput {
            title,
            started_at: started_at.to_rfc3339(),
            duration_ms: audio_duration_ms,
            language: language_for_meeting.unwrap_or_else(|| "auto".into()),
            dir_path,
            dialect: dialect_str.to_string(),
            primary_path: art.primary_file.display().to_string(),
        },
    )?;

    Ok(())
}

/// Bundled input for [`register_meeting_in_registry`]. Avoids an
/// 8-positional-arg signature (which is easy to misorder at call
/// sites and forces `#[allow(clippy::too_many_arguments)]`). Fields
/// map 1:1 to the underlying `MeetingDraft` + `NoteDraft` + the
/// `ended_at` / `duration_ms` overrides.
struct RegisterInput {
    title: String,
    started_at: String,
    duration_ms: u64,
    language: String,
    dir_path: String,
    dialect: String,
    primary_path: String,
}

/// Insert a fresh meeting + its initial note row into `storage`
/// **atomically**. Uses `Storage::create_meeting_with_note` which
/// wraps both inserts in a single transaction (real adapter) or
/// validates-all-then-commits (in-memory fake), so a note insert
/// failure rolls back the meeting and never leaves the registry in
/// a meeting-without-note state.
///
/// Pure trait calls — no FS-side DB open here — so callers may pass
/// any `Storage` impl (real `RusqliteStorage` from CLI, in-memory
/// fake from tests, or a future shared handle from a daemon-only-
/// writes design).
fn register_meeting_in_registry(
    storage: &dyn Storage,
    input: RegisterInput,
) -> Result<(), (u8, String)> {
    let meeting_id = uuid::Uuid::new_v4().simple().to_string();
    // Mark ended_at = started_at + duration so the registry row is
    // immediately "complete" (CLI flow has no live capture). On
    // parse failure (shouldn't happen — `started_at` came from
    // chrono's own `to_rfc3339`), fall back to leaving ended_at
    // unset so the row is still queryable.
    let ended_at = chrono::DateTime::parse_from_rfc3339(&input.started_at)
        .map(|dt| (dt + chrono::Duration::milliseconds(input.duration_ms as i64)).to_rfc3339())
        .ok();
    // Surface read failures explicitly. If the freshly-written
    // markdown can't be read back, the operator should know
    // (permissions, antivirus quarantine, disk flap) — silently
    // storing empty content makes debugging meeting search later
    // much harder.
    let content_md = std::fs::read_to_string(&input.primary_path)
        .map_err(|e| (3, format!("read notes file {}: {}", input.primary_path, e)))?;
    let (meeting, _note) = storage
        .create_meeting_with_note(
            MeetingDraft {
                id: meeting_id.clone(),
                title: input.title,
                started_at: input.started_at,
                language: input.language,
                dir_path: input.dir_path,
            },
            NoteDraft {
                id: uuid::Uuid::new_v4().simple().to_string(),
                meeting_id,
                dialect: input.dialect,
                content_md,
                primary_path: input.primary_path,
            },
            ended_at.as_deref(),
            Some(input.duration_ms),
        )
        .map_err(|e| (3, format!("register meeting+note: {e}")))?;
    tracing::info!(meeting = %meeting.id, "registered meeting in registry");
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

/// VAD-aware transcription. Loads audio once, runs VAD, merges
/// adjacent regions with gap < 5s, calls ASR per merged region with
/// `language` as the per-call hint (`None` triggers auto-detect),
/// stitches the per-region transcripts back together with absolute
/// timestamps. Returns one `Transcript` covering the whole audio.
fn transcribe_with_vad(
    audio: &std::path::Path,
    asr: &dyn AsrProvider,
    vad: &dyn Vad,
    language: Option<&str>,
) -> Result<panops_core::Transcript, (u8, String)> {
    let (samples, sample_rate) = panops_portable::audio::load_wav_mono16k(audio).map_err(|e| {
        tracing::error!(error = %e, "audio loading failed");
        (2, "audio decode failed".to_string())
    })?;
    let regions = vad.detect_speech(&samples, sample_rate).map_err(|e| {
        tracing::error!(error = %e, "vad detect_speech failed");
        (2, "vad failed".to_string())
    })?;
    let merged = panops_portable::audio::merge_adjacent_regions(regions, 5_000);

    let mut stitched: Vec<panops_core::Segment> = Vec::new();
    let mut stitched_model: Option<String> = None;
    let total_ms = (samples.len() as u64 * 1000) / u64::from(sample_rate);
    for region in merged.iter() {
        let start_sample = ((region.start_ms * u64::from(sample_rate)) / 1000) as usize;
        let end_sample = ((region.end_ms * u64::from(sample_rate)) / 1000) as usize;
        let start_sample = start_sample.min(samples.len());
        let end_sample = end_sample.min(samples.len());
        if start_sample >= end_sample {
            tracing::warn!(
                start_ms = region.start_ms,
                end_ms = region.end_ms,
                "degenerate VAD region after bounds clamping, skipping"
            );
            continue;
        }
        let chunk = &samples[start_sample..end_sample];
        let region_t = asr.transcribe(chunk, sample_rate, language).map_err(|e| {
            tracing::error!(error = %e, "asr transcribe failed");
            (2, "transcription failed".to_string())
        })?;
        if stitched_model.is_none() && !region_t.segments.is_empty() {
            stitched_model = Some(region_t.model.clone());
        }
        for mut seg in region_t.segments {
            seg.start_ms = (seg.start_ms + region.start_ms).min(total_ms);
            seg.end_ms = (seg.end_ms + region.start_ms).min(total_ms);
            stitched.push(seg);
        }
    }

    let final_model = stitched_model.unwrap_or_else(|| "vad-multilingual".to_string());
    Ok(panops_core::Transcript {
        schema_version: panops_core::Transcript::SCHEMA_VERSION,
        model: final_model,
        audio_path: audio.to_path_buf(),
        audio_duration_ms: total_ms,
        diarized: false,
        segments: stitched,
    })
}

fn transcribe(
    audio: &std::path::Path,
    model: Option<PathBuf>,
    language: Option<&str>,
) -> Result<panops_core::Transcript, (u8, String)> {
    if !audio.exists() {
        return Err((1, format!("audio file not found: {audio:?}")));
    }

    if std::env::var("PANOPS_FAKE_ASR").ok().as_deref() == Some("1") {
        // Fake path: TranscriptFileFake (samples-based) returns the
        // canned sidecar transcript regardless of VAD output. Wire a
        // KnownRegionsFake VAD so the pipeline still runs end-to-end.
        let canned = panops_core::conformance::fakes::read_canned_sidecar(audio);
        let asr = panops_core::conformance::fakes::TranscriptFileFake::with_canned(canned);
        let vad = panops_core::conformance::fakes::KnownRegionsFake::default();
        return transcribe_with_vad(audio, &asr, &vad, language);
    }

    let model_path = match model {
        Some(p) => p,
        None => default_model_path().map_err(|e| (3, e.to_string()))?,
    };
    let model_path =
        ensure_model(DEFAULT_MODEL_NAME, &model_path).map_err(|e| (3, e.to_string()))?;
    let asr = WhisperRsAsr::new(model_path).map_err(|e| (3, e.to_string()))?;

    let vad_path =
        panops_portable::model::default_vad_model_path().map_err(|e| (3, e.to_string()))?;
    let vad_path =
        panops_portable::model::ensure_vad_model(&vad_path).map_err(|e| (3, e.to_string()))?;
    let vad = panops_portable::WhisperVad::new(&vad_path).map_err(|e| (3, e.to_string()))?;

    transcribe_with_vad(audio, &asr, &vad, language)
}

fn diarize(audio: &std::path::Path) -> Result<Vec<panops_core::diar::SpeakerTurn>, (u8, String)> {
    let (seg, emb) = ensure_diar_models().map_err(|e| (3, e.to_string()))?;
    let diar = SherpaDiarizer::new(seg, emb).map_err(|e| (3, e.to_string()))?;
    diar.diarize(audio).map_err(|e| (2, e.to_string()))
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use panops_core::conformance::fakes::InMemoryStorage;

    /// Trait-level test: `register_meeting_in_registry` is now a pure
    /// `&dyn Storage` consumer, so we exercise it against the
    /// in-memory fake (no DB open, no FS write for the storage path).
    /// Verifies that the function inserts a meeting + an attached note
    /// + the meeting is "complete" (ended_at + duration_ms set).
    ///
    /// `RusqliteStorage` independently passes the same conformance
    /// suite (`crates/panops-portable/tests/conformance_rusqlite_storage.rs`),
    /// so this fake-backed test doubles as evidence that the real
    /// adapter behaves identically when the CLI calls it.
    #[test]
    fn register_meeting_inserts_meeting_and_note_rows() {
        let tmp = tempfile::tempdir().unwrap();
        // Only the `primary_path` needs to exist on disk — we read its
        // contents into the `note.content_md` field. Everything else
        // is pure storage trait calls.
        let notes_dir = tmp.path().join("audio-notes");
        std::fs::create_dir_all(&notes_dir).unwrap();
        let notes_md = notes_dir.join("notes.md");
        std::fs::write(&notes_md, "# Test notes\n").unwrap();

        let storage = InMemoryStorage::new();
        register_meeting_in_registry(
            &storage,
            RegisterInput {
                title: "Test Meeting".to_string(),
                started_at: "2026-05-05T10:00:00+00:00".to_string(),
                duration_ms: 60_000,
                language: "en".to_string(),
                dir_path: notes_dir.to_string_lossy().into_owned(),
                dialect: "basic".to_string(),
                primary_path: notes_md.to_string_lossy().into_owned(),
            },
        )
        .expect("registration should succeed");

        let rows = storage.list_meetings().unwrap();
        assert_eq!(rows.len(), 1, "expected one registered meeting");
        assert_eq!(rows[0].title, "Test Meeting");
        assert_eq!(rows[0].duration_ms, 60_000);

        let m = storage.get_meeting(&rows[0].id).unwrap();
        assert_eq!(m.language, "en");
        assert!(m.ended_at.is_some(), "ended_at should be set");
        assert_eq!(m.duration_ms, Some(60_000));

        let notes = storage.list_notes_for_meeting(&m.id).unwrap();
        assert_eq!(notes.len(), 1, "expected one note row");
        assert_eq!(notes[0].dialect, "basic");
        assert_eq!(notes[0].primary_path, notes_md.to_string_lossy());
        assert_eq!(notes[0].content_md, "# Test notes\n");
    }
}
