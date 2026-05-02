//! IPC server entry point. Owns the tokio runtimes and binds the UDS.
//!
//! Per-connection jsonrpsee serve over `tokio::net::UnixListener`. Two
//! control methods (`notes.generate`, `meeting.list`) and one
//! subscription (`events.subscribe`) are registered. `notes.generate`
//! routes through `spawn_blocking` to the rayon-driven pipeline; the
//! result emits as a `JobDone` / `JobError` event over WebSocket.
//!
//! Runtime topology (see slice 05 spec §"Runtime topology"): two
//! tokio runtimes — Runtime A drives jsonrpsee accept + dispatch,
//! Runtime B drives outbound LLM HTTP. Heavy adapter init
//! (`WhisperRsAsr`, `SherpaDiarizer`) takes ~20s and runs in a
//! `spawn_blocking` task on Runtime A *concurrent with* the accept
//! loop — see [`EngineServices::pending`] and [`init_heavy_adapters`].
//! Until init completes, `notes.generate` returns
//! `IpcError::ProviderUnavailable { message: "engine warming up; retry shortly" }`;
//! `meeting.list` is unaffected (returns `[]`, no heavy adapters
//! needed). This is the eager-after-bind shape that closes #74.

mod events;
mod handlers;
mod socket;

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

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

/// Heavy adapter trio loaded together — bundled so the `OnceLock` swap
/// is atomic (one set, all three present) and so `notes.generate`'s
/// readiness check is one branch instead of three.
pub(super) struct HeavyAdapters {
    pub(super) asr: Arc<dyn AsrProvider + Send + Sync>,
    pub(super) diar: Arc<dyn Diarizer + Send + Sync>,
    pub(super) exporter: Arc<dyn NotesExporter + Send + Sync>,
}

/// Wiring point for the IPC server. Two construction paths.
///
/// [`EngineServices::ready`] is synchronous: all adapters present at
/// return. Used by tests (`MockLlm` + `TranscriptFileFake` +
/// `KnownTurnsFake` + `FakeNotesExporter`) and by any caller that
/// already has the heavy trio constructed.
///
/// [`EngineServices::pending`] is for `serve`, where the heavy adapter
/// trio (`WhisperRsAsr`, `SherpaDiarizer`, `MarkdownExporter`) loads
/// multi-hundred-MB Whisper + diarization models and would push past
/// the integration test's 5s "socket appears" budget. Returns the
/// `OnceLock` handle so the background init task can fill it
/// concurrent with the accept loop coming up.
///
/// `llm` is exposed directly because `GenaiLlm::with_handle` is
/// instant (no model download); the heavy trio is hidden behind the
/// `OnceLock` so the readiness gate is a single `services.heavy.get()`
/// in `run_notes_pipeline`.
pub struct EngineServices {
    pub llm: Arc<dyn LlmProvider + Send + Sync>,
    pub(super) heavy: Arc<OnceLock<Result<HeavyAdapters, String>>>,
}

impl EngineServices {
    /// Construct with all adapters present. Pre-fills the `OnceLock` so
    /// `notes.generate` skips the warmup gate immediately. This is the
    /// shape every integration test uses.
    pub fn ready(
        llm: Arc<dyn LlmProvider + Send + Sync>,
        asr: Arc<dyn AsrProvider + Send + Sync>,
        diar: Arc<dyn Diarizer + Send + Sync>,
        exporter: Arc<dyn NotesExporter + Send + Sync>,
    ) -> Self {
        let heavy = Arc::new(OnceLock::new());
        let _ = heavy.set(Ok(HeavyAdapters {
            asr,
            diar,
            exporter,
        }));
        Self { llm, heavy }
    }

    /// Construct with only the (cheap) LLM adapter. Returns the
    /// `OnceLock` handle so the caller can spawn a background init
    /// task that resolves the heavy trio and fills the lock. Until
    /// the lock is set, `notes.generate` returns
    /// `IpcError::ProviderUnavailable`.
    ///
    /// `pub(crate)` — only `run_serve` (same crate) needs to construct
    /// the pending shape. Integration tests use [`Self::ready`].
    /// `HeavyAdapters` stays `pub(super)` so it can't leak out of
    /// `server::*`.
    pub(crate) fn pending(
        llm: Arc<dyn LlmProvider + Send + Sync>,
    ) -> (Self, Arc<OnceLock<Result<HeavyAdapters, String>>>) {
        let heavy = Arc::new(OnceLock::new());
        let services = Self {
            llm,
            heavy: heavy.clone(),
        };
        (services, heavy)
    }
}

/// CLI entry point — owns both tokio runtimes and the signal handler.
pub fn run_serve(socket: Option<PathBuf>) -> Result<(), (u8, String)> {
    let path = match socket {
        Some(p) => p,
        None => socket::default_socket_path().map_err(|e| (3, e))?,
    };

    // Runtime B: outbound LLM HTTP. Built first so its handle can be
    // cloned into the LLM adapter before Runtime A starts polling RPC.
    let llm_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("panops-llm-http")
        .build()
        .map_err(|e| (3, format!("build llm runtime: {e}")))?;
    let llm_handle = llm_rt.handle().clone();

    // Runtime A: jsonrpsee + UDS accept + heavy-adapter init.
    let rpc_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("panops-rpc")
        .build()
        .map_err(|e| (3, format!("build rpc runtime: {e}")))?;

    let result = rpc_rt.block_on(async move {
        // Build the LIGHT services first — just the LLM adapter (cheap,
        // instant). The heavy trio (Whisper / Sherpa / MarkdownExporter)
        // is filled by a `spawn_blocking` task that runs concurrent
        // with the accept loop, so the socket binds within the 5s
        // budget and `notes.generate` is gated with
        // `ProviderUnavailable` until the trio is ready.
        let llm = build_llm(llm_handle);
        let (services, heavy_lock) = EngineServices::pending(llm);
        spawn_heavy_init(heavy_lock);
        run_serve_in_process(&path, services, None).await
    });
    drop(llm_rt);

    // Force exit. The heavy-init `spawn_blocking` task may still be
    // running (Whisper/Sherpa model-load syscalls aren't cancellable);
    // tokio's blocking pool can't interrupt them, so without
    // `process::exit` the process stays alive past shutdown until the
    // load finishes. SIGTERM-to-exit must stay under the integration
    // test's 5s budget. Buffered tracing lines may be lost; that's
    // acceptable on shutdown.
    let exit_code = match result {
        Ok(()) => 0u8,
        Err((code, msg)) => {
            eprintln!("error: {msg}");
            code
        }
    };
    std::process::exit(i32::from(exit_code));
}

/// LLM adapter for `serve` mode. Picks a model based on environment
/// (mirrors `GenaiLlm::auto`'s precedence) and wires it to Runtime B
/// via `with_handle`. We don't delegate to `GenaiLlm::auto` directly
/// because `auto` uses the lazy shared CLI runtime — `serve` mode
/// must route outbound HTTP through Runtime B, not the CLI runtime.
/// Provider precedence stays in sync with `auto`; both should collapse
/// when `panops-portable::genai_llm` grows a `auto_with_handle` helper.
fn build_llm(handle: tokio::runtime::Handle) -> Arc<dyn LlmProvider + Send + Sync> {
    use panops_portable::genai_llm::GenaiLlm;
    let model = if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        "claude-haiku-4-5-20251001"
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        "gpt-4o-mini"
    } else {
        "gemma3:4b"
    };
    Arc::new(GenaiLlm::with_handle(model, handle))
}

/// Spawn the heavy-adapter init task. Runs on tokio's blocking pool
/// because `WhisperRsAsr::new` and `SherpaDiarizer::new` perform sync
/// I/O + model load + graph compile — sync work that mustn't land on
/// the RPC accept thread. On success the `OnceLock` holds
/// `Ok(HeavyAdapters)`; on failure it holds `Err(message)` so
/// `notes.generate` surfaces an explicit `IpcError::Internal` instead
/// of an indefinite "warming up" hang.
fn spawn_heavy_init(heavy_lock: Arc<OnceLock<Result<HeavyAdapters, String>>>) {
    tokio::task::spawn_blocking(move || {
        let result = init_heavy_adapters();
        match &result {
            Ok(_) => tracing::info!("heavy adapters ready"),
            Err(msg) => tracing::error!(error = %msg, "heavy adapter init failed"),
        }
        let _ = heavy_lock.set(result);
    });
}

/// Construct the real heavy adapter trio. Mirrors `panops-engine`'s
/// CLI default-mode + notes-mode wiring (`WhisperRsAsr`,
/// `SherpaDiarizer`, `MarkdownExporter`). String error type so the
/// failure can survive being stored in the `OnceLock` and surface as
/// `IpcError::Internal` to the wire — the operator still sees the
/// full detail via `tracing::error!` in [`spawn_heavy_init`].
fn init_heavy_adapters() -> Result<HeavyAdapters, String> {
    use panops_portable::markdown_exporter::MarkdownExporter;
    use panops_portable::model::{
        DEFAULT_MODEL_NAME, default_model_path, ensure_diar_models, ensure_model,
    };
    use panops_portable::{SherpaDiarizer, WhisperRsAsr};

    let model_path = default_model_path().map_err(|e| e.to_string())?;
    let model_path = ensure_model(DEFAULT_MODEL_NAME, &model_path).map_err(|e| e.to_string())?;
    let asr = WhisperRsAsr::new(model_path).map_err(|e| e.to_string())?;
    let (seg, emb) = ensure_diar_models().map_err(|e| e.to_string())?;
    let diar = SherpaDiarizer::new(seg, emb).map_err(|e| e.to_string())?;

    Ok(HeavyAdapters {
        asr: Arc::new(asr),
        diar: Arc::new(diar),
        exporter: Arc::new(MarkdownExporter),
    })
}

/// Async test entry point. Tests inject fakes via `services` (built
/// with [`EngineServices::ready`]) and trigger shutdown via the
/// optional `external_shutdown` watch receiver. When `external_shutdown`
/// is `None` the server installs its own SIGINT/SIGTERM handler and
/// runs until signalled.
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
    // Cap concurrent connections at 100. Slice 05 is single-user (one
    // Mac shell + the integration test harness opening 1-2 clients per
    // case), so 100 is two orders of magnitude over the realistic
    // ceiling. When the cap is hit, `try_acquire` returns `None` and
    // the per-request handler responds with HTTP 429 (`too_many_requests`)
    // rather than stalling — keeps a runaway client from exhausting the
    // accept loop. Revisit alongside the DoS bound on `notes.generate`
    // (filed as issue #85, severity:high).
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
