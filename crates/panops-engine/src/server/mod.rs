//! IPC server entry point. Owns the tokio runtimes and binds the UDS.
//!
//! Wave 4I: per-connection jsonrpsee serve over `tokio::net::UnixListener`.
//! Two control methods (`notes.generate`, `meeting.list`) and one
//! subscription (`events.subscribe`) are registered. `notes.generate`
//! is a stub until Wave 5K wires the pipeline + broadcast channel.
//!
//! Runtime topology (see slice 05 spec §"Runtime topology"): two
//! tokio runtimes — Runtime A drives jsonrpsee accept + dispatch,
//! Runtime B drives outbound LLM HTTP. Wave 5K's `spawn_blocking`
//! pipeline calls reach Runtime B via the stored `Handle`.

mod events;
mod handlers;
mod socket;

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use futures_util::FutureExt;
use jsonrpsee::Methods;
use jsonrpsee::core::middleware::RpcServiceBuilder;
use jsonrpsee::server::{
    ConnectionGuard, ConnectionState, ServerConfig, http, serve_with_graceful_shutdown,
    stop_channel, ws,
};
use panops_core::asr::AsrProvider;
use panops_core::diar::Diarizer;
use panops_core::exporter::NotesExporter;
use panops_core::llm::LlmProvider;
use tokio::sync::watch;

use crate::server::handlers::{IpcImpl, IpcServer};

/// Wiring point for slice-05 server tests AND the production CLI `serve`
/// path. Tests construct an `EngineServices` with fakes (`MockLlm`,
/// `TranscriptFileFake`, `KnownTurnsFake`, `FakeNotesExporter`); the CLI
/// wires adapters via [`stub_services`] — still fakes today (issue #74
/// tracks the real-adapter wiring deferred from Wave 5K).
pub struct EngineServices {
    pub llm: Arc<dyn LlmProvider + Send + Sync>,
    pub asr: Arc<dyn AsrProvider + Send + Sync>,
    pub diar: Arc<dyn Diarizer + Send + Sync>,
    pub exporter: Arc<dyn NotesExporter + Send + Sync>,
}

/// CLI entry point — owns both tokio runtimes and the signal handler.
pub fn run_serve(socket: Option<PathBuf>) -> Result<(), (u8, String)> {
    let path = match socket {
        Some(p) => p,
        None => socket::default_socket_path().map_err(|e| (3, e))?,
    };

    // Runtime B: outbound LLM HTTP. Built first so its handle can be
    // cloned into the LLM factory before Runtime A starts polling RPC.
    let llm_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("panops-llm-http")
        .build()
        .map_err(|e| (3, format!("build llm runtime: {e}")))?;
    let llm_handle = llm_rt.handle().clone();

    // Runtime A: jsonrpsee + UDS accept.
    let rpc_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("panops-rpc")
        .build()
        .map_err(|e| (3, format!("build rpc runtime: {e}")))?;

    let result = rpc_rt.block_on(async move {
        // Slice 05 Wave 5K: still using a *minimal* `EngineServices` so
        // the socket binds within the 5s budget that
        // `ipc_server_starts_and_binds` enforces. Real Whisper-large
        // load is ~20s and would blow that. The CLI smoke is manual
        // anyway; the in-process integration tests inject real adapters
        // directly through `run_serve_in_process`. Tracking real-adapter
        // wiring (lazy `OnceLock` or eager-after-bind) as issue #74.
        let services = stub_services(llm_handle);
        // No external shutdown: `run_serve_in_process` installs its
        // own signal handler before bind, so the SIGTERM-between-
        // bind-and-handler window from earlier waves is gone.
        run_serve_in_process(&path, services, None).await
    });
    drop(llm_rt);
    result
}

/// Async test entry point. Tests inject fakes via `services` and
/// trigger shutdown via the optional `external_shutdown` watch
/// receiver. When `external_shutdown` is `None` the server installs
/// its own SIGINT/SIGTERM handler and runs until signalled.
///
/// Shutdown is a `tokio::sync::watch::channel(bool)` end-to-end:
/// `watch` stores its current value, so even a receiver subscribed
/// after `send(true)` fires sees the shutdown. (The earlier `Notify`
/// shape lacked stored-permit semantics, leading to a lost-wakeup
/// race when `notify_waiters()` fired before the bridge task was
/// first polled.)
pub async fn run_serve_in_process(
    path: &Path,
    services: EngineServices,
    external_shutdown: Option<watch::Receiver<bool>>,
) -> Result<(), (u8, String)> {
    // Internal shutdown channel. Signal handler (installed below) and
    // optional external_shutdown both feed this single watch; the
    // accept loop selects on its receiver.
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    // Install signal handlers BEFORE bind. A SIGTERM that arrives
    // between bind and signal-handler-install would otherwise hit
    // tokio's default handler (process killed, no socket cleanup).
    // Installing first means: if registration fails we propagate the
    // error before any socket file exists, and any signal arriving
    // post-install is queued by tokio's per-signal stream.
    #[cfg(unix)]
    {
        install_signal_handler(shutdown_tx.clone())
            .map_err(|e| (3, format!("install signal handler: {e}")))?;
    }

    if let Some(mut external_rx) = external_shutdown {
        let tx = shutdown_tx.clone();
        tokio::spawn(async move {
            // If the external sender already fired before we got
            // here, the current value is `true` and we forward
            // immediately. Otherwise wait for the next change.
            if *external_rx.borrow() {
                let _ = tx.send(true);
                return;
            }
            while external_rx.changed().await.is_ok() {
                if *external_rx.borrow() {
                    let _ = tx.send(true);
                    break;
                }
            }
        });
    }

    let listener = match socket::bind_with_lifecycle(path).await {
        Ok(l) => l,
        Err(socket::BindError::EngineAlreadyRunning(p)) => {
            return Err((1, format!("engine already running at {}", p.display())));
        }
        Err(socket::BindError::Bind(m)) => return Err((3, m)),
    };
    tracing::info!(socket = ?path, "panops-engine serve listening");

    let (events_tx, _events_rx_keepalive) = events::channel();
    let services_arc = Arc::new(services);
    let ipc_impl = IpcImpl {
        services: services_arc,
        events_tx,
    };
    let methods: Methods = ipc_impl.into_rpc().into();

    // jsonrpsee-internal stop signal. Bridge the watch into it so
    // `serve_with_graceful_shutdown` drains in-flight RPCs before
    // returning.
    let (stop_handle, server_handle) = stop_channel();

    let mut bridge_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        // `changed()` returns immediately if the value already changed
        // before subscription — which means a late bridge subscriber
        // still sees the shutdown.
        while bridge_rx.changed().await.is_ok() {
            if *bridge_rx.borrow() {
                break;
            }
        }
        drop(server_handle);
    });

    let conn_id = Arc::new(AtomicU32::new(0));
    // Cap concurrent connections — slice 05 is single-user; 100 is
    // generous for the test client + Mac shell.
    let conn_guard = Arc::new(ConnectionGuard::new(100));

    let cleanup_path = path.to_path_buf();
    // If shutdown was already fired (e.g. immediately re-checked after
    // the bridge above), bail out early — no point opening the loop.
    if *shutdown_rx.borrow() {
        tracing::info!("shutdown was already pending at loop start");
        let _ = std::fs::remove_file(&cleanup_path);
        return Ok(());
    }
    loop {
        tokio::select! {
            biased;
            res = shutdown_rx.changed() => {
                if res.is_err() || *shutdown_rx.borrow() {
                    tracing::info!("shutdown signal received; breaking accept loop");
                    break;
                }
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let methods = methods.clone();
                        let stop_handle = stop_handle.clone();
                        let conn_id = conn_id.clone();
                        let conn_guard = conn_guard.clone();
                        let stop_handle_for_serve = stop_handle.clone();
                        let svc = tower::service_fn(move |req| {
                            let methods = methods.clone();
                            let stop_handle = stop_handle.clone();
                            let conn_guard_inner = conn_guard.clone();
                            let id = conn_id.fetch_add(1, Ordering::Relaxed);
                            async move {
                                let Some(conn_permit) = conn_guard_inner.try_acquire() else {
                                    return Ok::<_, Infallible>(http::response::too_many_requests());
                                };
                                let conn = ConnectionState::new(stop_handle.clone(), id, conn_permit);
                                let server_cfg = ServerConfig::default();

                                if ws::is_upgrade_request(&req) {
                                    let rpc_service = RpcServiceBuilder::new();
                                    match ws::connect(req, server_cfg, methods, conn, rpc_service).await {
                                        Ok((rp, conn_fut)) => {
                                            tokio::spawn(conn_fut);
                                            Ok(rp)
                                        }
                                        Err(rp) => Ok(rp),
                                    }
                                } else {
                                    let rpc_service = RpcServiceBuilder::new();
                                    let rp = http::call_with_service_builder(
                                        req,
                                        server_cfg,
                                        conn,
                                        methods,
                                        rpc_service,
                                    )
                                    .await;
                                    Ok(rp)
                                }
                            }
                            .boxed()
                        });

                        let stop_fut = stop_handle_for_serve.shutdown();
                        tokio::spawn(async move {
                            if let Err(e) = serve_with_graceful_shutdown(stream, svc, stop_fut).await {
                                tracing::warn!(error = ?e, "serve_with_graceful_shutdown error");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!(error = ?e, "accept error");
                    }
                }
            }
        }
    }

    tracing::info!("removing socket file and exiting run_serve_in_process");
    let _ = std::fs::remove_file(&cleanup_path);
    Ok(())
}

/// Slice 05 stand-in for `EngineServices`. Real `WhisperRsAsr` /
/// `SherpaDiarizer` constructors load multi-hundred-MB models eagerly,
/// which would push `serve` past the 5-second "socket appears" budget
/// integration tests rely on. Issue #74 tracks the follow-up — either
/// an eager-after-bind path (background-task adapter init) or per-call
/// lazy factories.
///
/// The placeholder uses `GenaiLlm::with_handle` (cheap; constructs
/// only the genai client) plus `panops-core`'s deterministic fakes
/// for ASR / diar / exporter. `notes.generate` against `panops-engine
/// serve` from a shell will therefore use fake ASR/diar — the
/// in-process integration tests inject real adapters directly through
/// `run_serve_in_process`, so the test surface isn't affected.
fn stub_services(llm_handle: tokio::runtime::Handle) -> EngineServices {
    use panops_core::conformance::fakes::{FakeNotesExporter, KnownTurnsFake, TranscriptFileFake};
    use panops_portable::genai_llm::GenaiLlm;

    // Same provider precedence as `GenaiLlm::auto`, but bound to
    // Runtime B's handle via `with_handle` so outbound HTTP doesn't
    // collide with Runtime A's RPC accept loop.
    let model = if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        "claude-haiku-4-5-20251001"
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        "gpt-4o-mini"
    } else {
        "gemma3:4b"
    };
    let llm = GenaiLlm::with_handle(model, llm_handle);

    EngineServices {
        llm: Arc::new(llm),
        asr: Arc::new(TranscriptFileFake),
        diar: Arc::new(KnownTurnsFake),
        exporter: Arc::new(FakeNotesExporter),
    }
}

/// Register OS-level SIGINT/SIGTERM handlers and spawn the waiter
/// task. Registration happens synchronously on the calling thread so
/// any signal arriving after this returns is queued by tokio's
/// per-signal `Signal` stream; the spawned waiter sees it on first
/// poll. Returns the registration error instead of panicking, so the
/// caller can surface a clean exit code without leaving a socket file
/// behind (registration is intentionally invoked BEFORE bind).
#[cfg(unix)]
fn install_signal_handler(shutdown: watch::Sender<bool>) -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::spawn(async move {
        tokio::select! {
            _ = sigint.recv() => { tracing::info!("SIGINT received"); }
            _ = sigterm.recv() => { tracing::info!("SIGTERM received"); }
        }
        tracing::info!("firing shutdown via watch channel");
        let _ = shutdown.send(true);
    });
    Ok(())
}
