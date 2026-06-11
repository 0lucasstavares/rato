mod client;
mod doctor;
mod install;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use serde_json::Value;

use rat_proto::{methods, Event, NewEvent};

/// RATO control CLI.
#[derive(Parser)]
#[command(name = "rat", version, about = "RATO control CLI")]
struct Cli {
    /// Daemon socket (default: $XDG_RUNTIME_DIR/rato/ratd.sock)
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show daemon status
    Status,
    /// Append an event (used by shell hooks and for testing)
    Emit {
        kind: String,
        #[arg(long, default_value = "cli")]
        source: String,
        /// JSON payload
        #[arg(long)]
        payload: Option<String>,
    },
    /// Inspect events
    Events {
        #[command(subcommand)]
        cmd: EventsCmd,
    },
    /// Install the user-level systemd service
    Install {
        /// Write the unit but do not run systemctl (for tests/CI)
        #[arg(long)]
        no_systemctl: bool,
        /// Explicit path to the ratd binary (default: sibling of this binary)
        #[arg(long)]
        ratd_path: Option<PathBuf>,
    },
    /// Check the local installation
    Doctor,
}

#[derive(Subcommand)]
enum EventsCmd {
    /// Show the most recent events
    Recent {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(rat_core::paths::socket_path);

    match cli.cmd {
        Cmd::Status => {
            let mut c = client::Client::connect(&socket).await?;
            let s = c.status().await?;
            println!("ratd {} (proto {})", s.version, s.proto_version);
            println!("uptime: {}s", s.uptime_ms / 1000);
            println!("events: {}", s.event_count);
            println!("db: {}", s.db_path);
        }
        Cmd::Emit { kind, source, payload } => {
            let payload: Value = match payload {
                Some(s) => serde_json::from_str(&s).context("--payload must be valid JSON")?,
                None => Value::Null,
            };
            let mut c = client::Client::connect(&socket).await?;
            let ev = NewEvent { kind, source, payload, ..Default::default() };
            let appended: Event = serde_json::from_value(
                c.call(methods::EVENTS_APPEND, serde_json::to_value(ev)?).await?,
            )?;
            println!("{} {} {}", appended.id, appended.ts, appended.kind);
        }
        Cmd::Events { cmd: EventsCmd::Recent { limit } } => {
            let mut c = client::Client::connect(&socket).await?;
            let events: Vec<Event> = serde_json::from_value(
                c.call(methods::EVENTS_RECENT, serde_json::json!({ "limit": limit })).await?,
            )?;
            for e in events {
                let payload =
                    if e.payload.is_null() { String::new() } else { e.payload.to_string() };
                println!("{}  {:<20} {:<10} {}", e.ts, e.kind, e.source, payload);
            }
        }
        Cmd::Install { no_systemctl, ratd_path } => install::install(no_systemctl, ratd_path)?,
        Cmd::Doctor => doctor::doctor(&socket).await?,
    }
    Ok(())
}
