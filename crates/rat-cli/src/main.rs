use rat_client as client;
mod doctor;
mod install;
mod shellinit;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};

use rat_proto::{methods, Event, ModeState, NewEvent, Observation, Project, WorkSession};

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
    /// Report a finished shell command (called by the shell hooks)
    EmitShell {
        #[arg(long)]
        cmd: String,
        #[arg(long)]
        cwd: String,
        #[arg(long)]
        exit: i32,
        #[arg(long, default_value_t = 0)]
        duration_ms: i64,
    },
    /// Print shell hook code: eval "$(rat shell-init bash)"
    ShellInit {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// List known projects
    Projects,
    /// Show recent work sessions
    Sessions {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Show recent observations
    Observations {
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        kind: Option<String>,
    },
    /// Show active/away mode
    Mode,
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

#[derive(Clone, Copy, ValueEnum)]
enum Shell {
    Bash,
    Zsh,
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
        Cmd::EmitShell { cmd, cwd, exit, duration_ms } => {
            let mut c = client::Client::connect(&socket).await?;
            let ev = NewEvent {
                kind: "shell_cmd".into(),
                source: "shell".into(),
                payload: json!({"cmd": cmd, "cwd": cwd, "exit": exit, "duration_ms": duration_ms}),
                ..Default::default()
            };
            // result may be null (loop guard) — that's fine, stay silent
            c.call(methods::EVENTS_APPEND, serde_json::to_value(ev)?).await?;
        }
        Cmd::ShellInit { shell } => print!("{}", shellinit::snippet(shell.into())?),
        Cmd::Projects => {
            let mut c = client::Client::connect(&socket).await?;
            let projects: Vec<Project> =
                serde_json::from_value(c.call(methods::PROJECTS_LIST, Value::Null).await?)?;
            for p in projects {
                println!("{:<24} {}", p.name, p.root_path);
            }
        }
        Cmd::Sessions { limit } => {
            let mut c = client::Client::connect(&socket).await?;
            let sessions: Vec<WorkSession> = serde_json::from_value(
                c.call(methods::SESSIONS_RECENT, json!({"limit": limit})).await?,
            )?;
            for s in sessions {
                let state = match s.ended {
                    Some(_) => "closed",
                    None => "open",
                };
                let mins = (s.last_activity - s.started) / 60_000;
                println!(
                    "{}  {:<8} {:>4} min  {:>4} cmds  project {}",
                    s.started, state, mins, s.commands, s.project_id
                );
            }
        }
        Cmd::Observations { limit, kind } => {
            let mut c = client::Client::connect(&socket).await?;
            let obs: Vec<Observation> = serde_json::from_value(
                c.call(methods::OBSERVATIONS_RECENT, json!({"limit": limit, "kind": kind})).await?,
            )?;
            for o in obs {
                println!("{}  {:<20} {}", o.ts, o.kind, o.content.replace('\n', "\\n"));
            }
        }
        Cmd::Mode => {
            let mut c = client::Client::connect(&socket).await?;
            let m: ModeState = serde_json::from_value(c.call(methods::MODE_GET, Value::Null).await?)?;
            match m.idle_ms {
                Some(idle) => println!("{} (idle {}s)", m.mode, idle / 1000),
                None => println!("{} (idle unknown)", m.mode),
            }
        }
        Cmd::Install { no_systemctl, ratd_path } => install::install(no_systemctl, ratd_path)?,
        Cmd::Doctor => doctor::doctor(&socket).await?,
    }
    Ok(())
}
