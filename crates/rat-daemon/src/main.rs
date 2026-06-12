use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::Parser;
use serde_json::json;
use tracing_subscriber::EnvFilter;

use rat_brain::backend::{BackendConfig, Provider, make_backend};
use rat_brain::keys;
use rat_brain::critic::Critic;
use rat_core::clock::{Clock, SystemClock};
use rat_core::paths;
use rat_daemon::capture::run_capture_tick;
use rat_daemon::config::Config;
use rat_daemon::ingest::Ingest;
use rat_daemon::memory_searcher::DaemonMemorySearcher;
use rat_daemon::mode::{self, ModeManager};
use rat_daemon::pins::{KeyringPinKeyStore, PinService};
use rat_daemon::server::{LlmStatusState, serve, ServerCtx};
use rat_workbench::runner::TaskRunner;
use rat_workbench::tmux::Tmux;
use rat_daemon::sessionizer::{Sessionizer, DEFAULT_GAP_MS};
use rat_memory::embed::EmbeddingClient;
use rat_proto::NewEvent;
use rat_ring::{RingKey, RingWriter};
use rat_sensors::gitwatch::{self, HeadState};
use rat_sensors::proc::{proc_detail, scan_procs, ProcWatcher, ALLOWLIST};
use rat_store::store::Store;

/// RATO daemon: observes, remembers, critiques.
#[derive(Parser)]
#[command(name = "ratd", version)]
struct Args {
    /// Socket path (default: $XDG_RUNTIME_DIR/rato/ratd.sock)
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Database path (default: ~/.local/share/rato/rato.db)
    #[arg(long)]
    db: Option<PathBuf>,
    /// Disable sensors (clipboard/proc/git/idle) — RPC-only mode for tests
    #[arg(long)]
    no_sensors: bool,
    /// Disable critic (LLM ticks) — for tests or when no LLM key is configured
    #[arg(long)]
    no_critic: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("RAT_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let socket = args.socket.unwrap_or_else(paths::socket_path);
    let db = args.db.unwrap_or_else(paths::db_path);

    if let Some(dir) = socket.parent() {
        paths::ensure_private_dir(dir).context("creating runtime dir")?;
    }
    if let Some(dir) = db.parent() {
        paths::ensure_private_dir(dir).context("creating data dir")?;
    }

    // Stale-socket handling: if something answers, another ratd is running.
    if socket.exists() {
        match tokio::net::UnixStream::connect(&socket).await {
            Ok(_) => anyhow::bail!("ratd already running on {}", socket.display()),
            Err(_) => {
                tracing::info!("removing stale socket {}", socket.display());
                std::fs::remove_file(&socket)?;
            }
        }
    }

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let store = Store::open(&db, clock.clone()).context("opening event store")?;
    let listener = tokio::net::UnixListener::bind(&socket)
        .with_context(|| format!("binding {}", socket.display()))?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;

    // Re-adopt sessions left open by the previous run.
    let mut sessionizer = Sessionizer::new(DEFAULT_GAP_MS);
    let open = store.open_sessions().await?;
    if !open.is_empty() {
        tracing::info!("re-adopting {} open work session(s)", open.len());
        sessionizer.preload(&open);
    }
    let ingest = Arc::new(Ingest::new(store.clone(), clock.clone(), sessionizer));
    let mode = Arc::new(ModeManager::new(clock.now_ms()));

    ingest
        .ingest(NewEvent { kind: "daemon_started".into(), source: "ratd".into(), ..Default::default() })
        .await?;

    if !args.no_sensors {
        spawn_sensors(store.clone(), ingest.clone(), mode.clone(), clock.clone());
    } else {
        tracing::info!("sensors disabled (--no-sensors)");
    }

    // Periodic sessionizer tick: close sessions gone silent past the gap
    {
        let ingest = ingest.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if let Err(e) = ingest.tick().await {
                    tracing::warn!("sessionizer tick failed: {e}");
                }
            }
        });
    }

    // --- Config + LLM setup ---
    let config_path = Config::default_path();
    let config = Config::load_or_init(&config_path);

    let provider = match config.llm.provider.as_str() {
        "anthropic" => Provider::Anthropic,
        "openrouter" => Provider::OpenRouter,
        _ => Provider::OpenAi,
    };

    let llm_key = keys::get_key(provider.clone()).ok();
    let backend: Option<Box<dyn rat_brain::backend::ChatBackend>> =
        llm_key.as_ref().map(|key| {
            make_backend(
                &BackendConfig {
                    provider: provider.clone(),
                    base_url: None,
                    critic_model: config.llm.critic_model.clone(),
                    cheap_model: config.llm.cheap_model.clone(),
                },
                key.clone(),
            )
        });

    // EmbeddingClient (OpenAI embeddings regardless of critic provider)
    let openai_key = std::env::var("RATO_OPENAI_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| keys::get_key(Provider::OpenAi).ok());
    let embedder: Option<EmbeddingClient> = openai_key.as_ref().map(|key| {
        EmbeddingClient::new("https://api.openai.com".to_string(), key.clone())
    });

    let critic_enabled = config.critic.enabled && !args.no_critic && backend.is_some();

    let llm_status = Arc::new(LlmStatusState {
        provider: config.llm.provider.clone(),
        embedding_enabled: std::sync::atomic::AtomicBool::new(embedder.is_some()),
        critic_enabled,
        last_error: std::sync::Mutex::new(None),
        openai_key: keys::key_present(Provider::OpenAi),
        anthropic_key: keys::key_present(Provider::Anthropic),
        openrouter_key: keys::key_present(Provider::OpenRouter),
    });

    if critic_enabled {
        let backend_box = backend.unwrap(); // safe since critic_enabled requires it
        let memory_searcher: Option<Box<dyn rat_brain::critic::MemorySearcher>> =
            Some(Box::new(DaemonMemorySearcher { embedder: embedder.clone(), llm_status: llm_status.clone() }));
        let critic = Arc::new(Critic::new(
            store.clone(),
            backend_box,
            memory_searcher,
            clock.clone(),
        ));

        // Combined fast+slow tick loop
        {
            let critic = critic.clone();
            let llm_status_tick = llm_status.clone();
            let fast_s = config.critic.fast_tick_s;
            let slow_s = config.critic.slow_tick_s;
            tokio::spawn(async move {
                let mut fast_interval =
                    tokio::time::interval(Duration::from_secs(fast_s));
                fast_interval
                    .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let mut slow_interval =
                    tokio::time::interval(Duration::from_secs(slow_s));
                slow_interval
                    .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tokio::select! {
                        _ = fast_interval.tick() => {
                            let signals = critic.fast_tick().await;
                            if !signals.is_empty() {
                                // The periodic slow loop may re-run on the same evidence; the 24h dedupe in the store absorbs duplicate pushbacks.
                                match critic.slow_tick(&signals).await {
                                    Some(pb) => {
                                        tracing::info!("critic: pushback created {}", pb.id);
                                        if let Ok(mut e) = llm_status_tick.last_error.lock() {
                                            *e = None;
                                        }
                                    }
                                    None => {
                                        if let Ok(mut e) = llm_status_tick.last_error.lock() {
                                            *e = None;
                                        }
                                    }
                                }
                            }
                        }
                        _ = slow_interval.tick() => {
                            let signals = critic.fast_tick().await;
                            if !signals.is_empty() {
                                // The periodic slow loop may re-run on the same evidence; the 24h dedupe in the store absorbs duplicate pushbacks.
                                if let Some(pb) = critic.slow_tick(&signals).await {
                                    tracing::info!("critic slow_tick: pushback {}", pb.id);
                                }
                                if let Ok(mut e) = llm_status_tick.last_error.lock() {
                                    *e = None;
                                }
                            }
                        }
                    }
                }
            });
        }

        // Hourly consolidation jobs loop
        {
            let store_h = store.clone();
            let embedder_h = embedder.clone();
            let clock_h = clock.clone();
            let llm_status_h = llm_status.clone();
            // Create a second backend for hourly (can't share the one moved into Critic)
            let hourly_backend: Option<Box<dyn rat_brain::backend::ChatBackend>> =
                llm_key.as_ref().map(|key| {
                    make_backend(
                        &BackendConfig {
                            provider: provider.clone(),
                            base_url: None,
                            critic_model: config.llm.critic_model.clone(),
                            cheap_model: config.llm.cheap_model.clone(),
                        },
                        key.clone(),
                    )
                });
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(3600));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    interval.tick().await;
                    // embedding may have been disabled at runtime (4xx from the API)
                    let embedder_now = embedder_h
                        .as_ref()
                        .filter(|_| llm_status_h.embedding_enabled.load(std::sync::atomic::Ordering::Relaxed));
                    if let Err(e) = rat_memory::jobs::hourly(
                        &store_h,
                        hourly_backend.as_deref(),
                        embedder_now,
                        &clock_h,
                    )
                    .await
                    {
                        tracing::warn!("hourly job error: {e}");
                        if let Ok(mut last_error) = llm_status_h.last_error.lock() {
                            *last_error = Some(e.to_string());
                        }
                    }
                }
            });
        }

        // Nightly loop (next 03:30 UTC, then every 24h)
        {
            let store_n = store.clone();
            let clock_n = clock.clone();
            let llm_status_n = llm_status.clone();
            // Create a backend for nightly day-summary generation (mirrors the hourly backend)
            let nightly_backend: Option<Box<dyn rat_brain::backend::ChatBackend>> =
                llm_key.as_ref().map(|key| {
                    make_backend(
                        &BackendConfig {
                            provider: provider.clone(),
                            base_url: None,
                            critic_model: config.llm.critic_model.clone(),
                            cheap_model: config.llm.cheap_model.clone(),
                        },
                        key.clone(),
                    )
                });
            tokio::spawn(async move {
                let delay_secs = secs_until_0330();
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                loop {
                    if let Err(e) =
                        rat_memory::jobs::nightly(&store_n, nightly_backend.as_deref(), &clock_n).await
                    {
                        tracing::warn!("nightly job error: {e}");
                        if let Ok(mut last_error) = llm_status_n.last_error.lock() {
                            *last_error = Some(e.to_string());
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(86400)).await;
                }
            });
        }
    } else {
        tracing::info!(
            "critic disabled (--no-critic={}, key_present={}, config.enabled={})",
            args.no_critic,
            backend.is_some(),
            config.critic.enabled
        );
    }

    tracing::info!("ratd {} listening on {}", env!("CARGO_PKG_VERSION"), socket.display());
    tracing::info!("event store at {}", db.display());

    let task_runner = TaskRunner::new(store.clone(), Tmux::new("rato"), clock.clone());

    let ring_dir = paths::state_dir().join("ring");
    let pins_dir = paths::data_dir().join("pins");
    paths::ensure_private_dir(&ring_dir).context("creating ring state dir")?;
    paths::ensure_private_dir(&pins_dir).context("creating pins data dir")?;
    let ring_key = Arc::new(RingKey::ephemeral());
    let screen_ring = Arc::new(RingWriter {
        dir: ring_dir,
        segment_secs: 10,
        ttl_secs: 20 * 60,
        clock: clock.clone(),
    });
    let pin_service = PinService::new(
        store.clone(),
        screen_ring.clone(),
        ring_key.clone(),
        Arc::new(KeyringPinKeyStore),
        pins_dir,
        clock.clone(),
    );

    if !args.no_sensors {
        spawn_capture_loop(store.clone(), screen_ring.clone(), ring_key.clone(), pin_service.clone());
    }

    // Approval expiry sweep: expire pending approvals every 60s
    {
        let store_exp = store.clone();
        let clock_exp = clock.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let now = clock_exp.now_ms();
                match store_exp.expire_approvals(now).await {
                    Ok(n) if n > 0 => tracing::info!("expired {n} approval(s)"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("expire_approvals failed: {e}"),
                }
            }
        });
    }

    // Agent-run poll sweep: advance running → done/failed every 3s
    {
        let runner_poll = task_runner.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let runs = match runner_poll.store.recent_agent_runs(100).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("poll sweep: recent_agent_runs failed: {e}");
                        continue;
                    }
                };
                for run in runs.into_iter().filter(|r| r.status == "running") {
                    if let Err(e) = runner_poll.poll(&run.id).await {
                        tracing::debug!("poll sweep: poll({}) failed: {e}", run.id);
                    }
                }
            }
        });
    }

    let ctx = Arc::new(ServerCtx {
        store,
        ingest,
        mode,
        started: Instant::now(),
        db_path: db,
        clock,
        embedder,
        llm_status,
        task_runner,
        pins: Some(pin_service),
    });

    tokio::select! {
        _ = serve(listener, ctx) => {}
        _ = shutdown_signal() => {
            tracing::info!("shutting down");
        }
    }

    let _ = std::fs::remove_file(&socket);
    Ok(())
}

fn spawn_sensors(store: Store, ingest: Arc<Ingest>, mode: Arc<ModeManager>, clock: Arc<dyn Clock>) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<NewEvent>(256);

    // pump: sensor events → mode activity + ingest
    {
        let ingest = ingest.clone();
        let mode = mode.clone();
        let clock = clock.clone();
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                mode.note_activity(clock.now_ms());
                if let Err(e) = ingest.ingest(ev).await {
                    tracing::warn!("sensor event ingest failed: {e}");
                }
            }
        });
    }

    rat_sensors::clipboard::spawn(tx.clone());

    // process watcher: 5 s scan/diff of allowlisted dev processes
    {
        let tx = tx.clone();
        let clock = clock.clone();
        tokio::spawn(async move {
            let allow: HashSet<&str> = ALLOWLIST.iter().copied().collect();
            let mut watcher = ProcWatcher::new();
            // baseline scan: track what's already running without emitting events
            watcher.observe(&scan_procs(&allow), clock.now_ms());
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let snap = scan_procs(&allow);
                let mut events = watcher.observe(&snap, clock.now_ms());
                for ev in events.iter_mut() {
                    if ev.kind == "proc_started" {
                        let pid = ev.payload["pid"].as_u64().unwrap_or(0) as u32;
                        let (cmdline, cwd) = proc_detail(pid);
                        if let Some(obj) = ev.payload.as_object_mut() {
                            if let Some(c) = cmdline {
                                obj.insert("cmdline".into(), json!(c));
                            }
                            if let Some(c) = cwd {
                                obj.insert("cwd".into(), json!(c));
                            }
                        }
                    }
                }
                for ev in events {
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                }
            }
        });
    }

    // git watcher: 10 s HEAD polling for recently-active projects
    {
        let ingest = ingest.clone();
        let clock = clock.clone();
        tokio::spawn(async move {
            const ACTIVE_WINDOW_MS: i64 = 24 * 3600 * 1000;
            let mut heads: HashMap<String, HeadState> = HashMap::new();
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let projects = match store.list_projects().await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("gitwatch: list_projects failed: {e}");
                        continue;
                    }
                };
                let now = clock.now_ms();
                for p in projects.into_iter().filter(|p| now - p.last_seen < ACTIVE_WINDOW_MS) {
                    let Some(head) = gitwatch::read_head(Path::new(&p.root_path)) else { continue };
                    let first_sighting = !heads.contains_key(&p.id);
                    let changed = heads.get(&p.id) != Some(&head);
                    if changed {
                        heads.insert(p.id.clone(), head.clone());
                        if first_sighting {
                            continue; // baseline, not a checkout
                        }
                        let ev = NewEvent {
                            kind: "git_head".into(),
                            source: "git".into(),
                            payload: json!({
                                "branch": head.branch,
                                "commit": head.commit,
                                "cwd": p.root_path,
                            }),
                            ..Default::default()
                        };
                        if let Err(e) = ingest.ingest(ev).await {
                            tracing::warn!("gitwatch ingest failed: {e}");
                        }
                    }
                }
            }
        });
    }

    // idle probe → away mode
    tokio::spawn(mode::run(mode, ingest, clock));
}

fn spawn_capture_loop(
    store: Store,
    ring: Arc<RingWriter>,
    ring_key: Arc<RingKey>,
    pins: PinService,
) {
    tokio::spawn(async move {
        #[cfg(feature = "screencast")]
        let source = rat_vision::screen_portal::PortalScreenSource::new();
        #[cfg(not(feature = "screencast"))]
        let source = rat_vision::screen::FakeScreenSource::new(vec![]);

        #[cfg(feature = "ocr")]
        let ocr = rat_vision::ocr_tesseract::TesseractOcr::new();
        #[cfg(not(feature = "ocr"))]
        let ocr = rat_vision::ocr::NullOcr;

        let mut pipeline = rat_vision::pipeline::CapturePipeline::new(source, ocr);
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(e) =
                run_capture_tick(&mut pipeline, ring.as_ref(), ring_key.as_ref(), &store, Some(&pins)).await
            {
                tracing::warn!("capture tick failed: {e}");
            }
            if let Err(e) = ring.prune() {
                tracing::debug!("ring prune failed: {e}");
            }
        }
    });
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

/// Compute seconds until next 03:30 UTC (simple modulo arithmetic).
/// DST/leap imprecision is acceptable.
fn secs_until_0330() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_secs =
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    secs_until_0330_from(now_secs)
}

/// Pure helper used by tests: compute seconds from `now_secs` (Unix epoch) until
/// the next 03:30 UTC boundary.
fn secs_until_0330_from(now_secs: u64) -> u64 {
    // 03:30 UTC = 3*3600 + 30*60 = 12600 seconds from midnight
    let target_secs_in_day: u64 = 3 * 3600 + 30 * 60;
    let secs_in_day = 86400u64;
    let current_secs_in_day = now_secs % secs_in_day;
    if current_secs_in_day < target_secs_in_day {
        target_secs_in_day - current_secs_in_day
    } else {
        secs_in_day - current_secs_in_day + target_secs_in_day
    }
}

#[cfg(test)]
mod tests {
    use super::secs_until_0330_from;

    // 03:30 UTC = 12600 seconds into the day.
    // Use a fixed day: 2024-01-01 00:00:00 UTC = 1704067200
    const DAY_START: u64 = 1704067200;
    const TARGET: u64 = 12600; // seconds into day for 03:30

    #[test]
    fn just_before_0330() {
        // 1 second before 03:30
        let now = DAY_START + TARGET - 1;
        assert_eq!(secs_until_0330_from(now), 1);
    }

    #[test]
    fn exactly_0330() {
        // exactly at 03:30 — next occurrence is 24h later
        let now = DAY_START + TARGET;
        assert_eq!(secs_until_0330_from(now), 86400);
    }

    #[test]
    fn just_after_0330() {
        // 1 second after 03:30 — next is nearly 24h away
        let now = DAY_START + TARGET + 1;
        assert_eq!(secs_until_0330_from(now), 86400 - 1);
    }
}
