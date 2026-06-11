use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use rat_core::clock::SystemClock;
use rat_core::paths;
use rat_daemon::server::{serve, ServerCtx};
use rat_proto::NewEvent;
use rat_store::store::Store;

/// RATO daemon: observes, remembers, critiques. M0: event spine + RPC.
#[derive(Parser)]
#[command(name = "ratd", version)]
struct Args {
    /// Socket path (default: $XDG_RUNTIME_DIR/rato/ratd.sock)
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Database path (default: ~/.local/share/rato/rato.db)
    #[arg(long)]
    db: Option<PathBuf>,
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

    let store = Store::open(&db, Arc::new(SystemClock)).context("opening event store")?;
    let listener = tokio::net::UnixListener::bind(&socket)
        .with_context(|| format!("binding {}", socket.display()))?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;

    store
        .append(NewEvent {
            kind: "daemon_started".into(),
            source: "ratd".into(),
            ..Default::default()
        })
        .await?;

    tracing::info!("ratd {} listening on {}", env!("CARGO_PKG_VERSION"), socket.display());
    tracing::info!("event store at {}", db.display());

    let ctx = Arc::new(ServerCtx { store, started: Instant::now(), db_path: db });

    tokio::select! {
        _ = serve(listener, ctx) => {}
        _ = shutdown_signal() => {
            tracing::info!("shutting down");
        }
    }

    let _ = std::fs::remove_file(&socket);
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
