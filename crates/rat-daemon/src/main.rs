use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::Parser;
use serde_json::json;
use tracing_subscriber::EnvFilter;

use rat_core::clock::{Clock, SystemClock};
use rat_core::paths;
use rat_daemon::ingest::Ingest;
use rat_daemon::mode::{self, ModeManager};
use rat_daemon::server::{serve, ServerCtx};
use rat_daemon::sessionizer::{Sessionizer, DEFAULT_GAP_MS};
use rat_proto::NewEvent;
use rat_sensors::gitwatch::{self, HeadState};
use rat_sensors::proc::{proc_detail, scan_procs, ProcWatcher, ALLOWLIST};
use rat_store::store::Store;

/// RATO daemon: observes, remembers, critiques. M1: cheap sensors.
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

    // periodic sessionizer tick: close sessions gone silent past the gap
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

    tracing::info!("ratd {} listening on {}", env!("CARGO_PKG_VERSION"), socket.display());
    tracing::info!("event store at {}", db.display());

    let ctx = Arc::new(ServerCtx { store, ingest, mode, started: Instant::now(), db_path: db });

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

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
