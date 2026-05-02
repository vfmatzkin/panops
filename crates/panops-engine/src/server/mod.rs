//! IPC server entry point. Owns the tokio runtime and binds the UDS.
//!
//! Slice 05 Wave 3G: socket lifecycle only. Wave 4I plugs in jsonrpsee.

mod socket;

use std::path::PathBuf;
use std::sync::Arc;

use panops_core::asr::AsrProvider;
use panops_core::diar::Diarizer;
use panops_core::exporter::NotesExporter;
use panops_core::llm::LlmProvider;
use tokio::sync::Notify;

/// Wiring point for slice-05 server tests AND the production CLI `serve`
/// path. Tests construct an `EngineServices` with fakes (`MockLlm`,
/// `TranscriptFileFake`, `KnownTurnsFake`, `FakeNotesExporter`); the CLI
/// wires real adapters.
pub struct EngineServices {
    pub llm: Arc<dyn LlmProvider + Send + Sync>,
    pub asr: Arc<dyn AsrProvider + Send + Sync>,
    pub diar: Arc<dyn Diarizer + Send + Sync>,
    pub exporter: Arc<dyn NotesExporter + Send + Sync>,
}

pub fn run_serve(socket: Option<PathBuf>) -> Result<(), (u8, String)> {
    let path = match socket {
        Some(p) => p,
        None => socket::default_socket_path().map_err(|e| (3, e))?,
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("panops-rpc")
        .build()
        .map_err(|e| (3, format!("build rpc runtime: {e}")))?;

    rt.block_on(async {
        let listener = match socket::bind_with_lifecycle(&path).await {
            Ok(l) => l,
            Err(socket::BindError::EngineAlreadyRunning(p)) => {
                return Err((1, format!("engine already running at {}", p.display())));
            }
            Err(socket::BindError::Bind(m)) => return Err((3, m)),
        };
        tracing::info!(socket = ?path, "panops-engine serve listening");

        let shutdown = Arc::new(Notify::new());
        spawn_signal_handler(shutdown.clone());

        // Slice 05 Wave 3G stub: accept-and-immediately-close.
        // Wave 4I replaces this loop with the jsonrpsee per-connection serve.
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((_stream, _addr)) => {
                            tracing::debug!("accepted (Wave 3G stub closes immediately)");
                        }
                        Err(e) => {
                            tracing::warn!(error = ?e, "accept error");
                        }
                    }
                }
                _ = shutdown.notified() => {
                    tracing::info!("shutdown signal received");
                    break;
                }
            }
        }

        let _ = std::fs::remove_file(&path);
        Ok::<_, (u8, String)>(())
    })?;
    Ok(())
}

#[cfg(unix)]
fn spawn_signal_handler(shutdown: Arc<Notify>) {
    tokio::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
        }
        shutdown.notify_waiters();
    });
}
