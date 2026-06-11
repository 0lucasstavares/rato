use rat_client as client;
mod doctor;
mod install;
mod shellinit;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};

use rat_proto::{
    methods, Event, HitDto, ModeState, NewEvent, Observation, Project, PushbackDto, WorkSession,
};

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
    /// Import API keys from key files and set the LLM provider
    Setup {
        #[arg(long, default_value = "openai", value_parser = ["openai", "anthropic", "openrouter"])]
        provider: String,
        /// Directory containing key files (default: ~/rato/keys)
        #[arg(long)]
        keys_dir: Option<PathBuf>,
    },
    /// Search memory and observations
    Search {
        query: String,
        #[arg(short, default_value_t = 8)]
        n: u32,
    },
    /// Show and manage pushbacks
    Pushbacks {
        #[arg(long, default_value_t = 10)]
        n: u32,
        #[command(subcommand)]
        cmd: Option<PushbacksCmd>,
    },
    /// Show LLM / critic configuration status
    LlmStatus,
}

#[derive(Subcommand)]
enum PushbacksCmd {
    /// Record feedback for a pushback
    Feedback {
        id: String,
        /// "useful" | "dismiss" | "snooze"
        verdict: String,
    },
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

const KEY_FILE_MAX_BYTES: u64 = 4096;

/// Reads key files from `keys_dir`. Returns `(keys, notes)` where `keys` maps
/// provider → trimmed key and `notes` carries human-readable warnings (e.g. for
/// oversized or unreadable files). Files larger than 4096 bytes are skipped and
/// a note is appended instead of storing potentially garbage content.
/// UTF-8 BOM (`\u{feff}`) is stripped before trimming.
///
/// Files: antr_k.txt → anthropic, open_k.txt → openai, openr_k.txt → openrouter.
pub fn read_key_files(
    keys_dir: &std::path::Path,
) -> (std::collections::HashMap<String, String>, Vec<String>) {
    let mapping = [
        ("antr_k.txt", "anthropic"),
        ("open_k.txt", "openai"),
        ("openr_k.txt", "openrouter"),
    ];
    let mut result = std::collections::HashMap::new();
    let mut notes = Vec::new();
    for (file, provider) in &mapping {
        let path = keys_dir.join(file);
        // Check file size before reading to avoid storing oversized garbage.
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > KEY_FILE_MAX_BYTES => {
                let msg = format!(
                    "skipped {} ({} bytes > {} byte limit)",
                    path.display(),
                    meta.len(),
                    KEY_FILE_MAX_BYTES
                );
                notes.push(msg);
                continue;
            }
            Err(_) => continue, // file does not exist or is unreadable — skip silently
            Ok(_) => {}
        }
        if let Ok(contents) = std::fs::read_to_string(&path) {
            // Strip UTF-8 BOM then trim surrounding whitespace.
            let trimmed = contents.trim_start_matches('\u{feff}').trim().to_string();
            if !trimmed.is_empty() {
                result.insert(provider.to_string(), trimmed);
            }
        }
    }
    (result, notes)
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
        Cmd::Setup { provider, keys_dir } => {
            let keys_dir = keys_dir.unwrap_or_else(|| {
                std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join("rato")
                    .join("keys")
            });
            let (keys, notes) = read_key_files(&keys_dir);
            for note in &notes {
                println!("note: {}", note);
            }
            if keys.is_empty() {
                println!("(no key files found in {})", keys_dir.display());
            }
            for (prov, value) in &keys {
                let p = match prov.as_str() {
                    "anthropic" => rat_brain::backend::Provider::Anthropic,
                    "openrouter" => rat_brain::backend::Provider::OpenRouter,
                    _ => rat_brain::backend::Provider::OpenAi,
                };
                match rat_brain::keys::set_key(p, value) {
                    Ok(()) => println!("stored rato/{} ({} chars)", prov, value.len()),
                    Err(e) => eprintln!("failed to store {}: {}", prov, e),
                }
            }
            // Write config with chosen provider
            let config_path = rat_daemon::config::Config::default_path();
            let mut config = rat_daemon::config::Config::load_or_init(&config_path);
            config.llm.provider = provider.clone();
            if let Ok(contents) = toml::to_string(&config) {
                if let Some(dir) = config_path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&config_path, contents);
            }
            println!("provider set to {}", provider);
        }
        Cmd::Search { query, n } => {
            let mut c = client::Client::connect(&socket).await?;
            let hits: Vec<HitDto> = serde_json::from_value(
                c.call(
                    methods::MEMORY_SEARCH,
                    serde_json::json!({ "query": query, "n": n }),
                )
                .await?,
            )?;
            if hits.is_empty() {
                println!("(no results)");
            }
            for h in hits {
                println!("{:<12} {:.4}  {}", h.kind, h.score, h.id);
            }
        }
        Cmd::Pushbacks { n, cmd: None } => {
            let mut c = client::Client::connect(&socket).await?;
            let pbs: Vec<PushbackDto> = serde_json::from_value(
                c.call(methods::PUSHBACKS_RECENT, serde_json::json!({ "n": n })).await?,
            )?;
            if pbs.is_empty() {
                println!("(no pushbacks)");
            }
            for pb in pbs {
                println!(
                    "{}  [{:<10}]  {:<15}  {}",
                    pb.ts, pb.status, pb.severity, pb.title
                );
            }
        }
        Cmd::Pushbacks { cmd: Some(PushbacksCmd::Feedback { id, verdict }), .. } => {
            let mut c = client::Client::connect(&socket).await?;
            let pb: PushbackDto = serde_json::from_value(
                c.call(
                    methods::PUSHBACKS_FEEDBACK,
                    serde_json::json!({ "id": id, "verdict": verdict }),
                )
                .await?,
            )?;
            println!("feedback recorded: {} → {}", pb.id, pb.status);
        }
        Cmd::LlmStatus => {
            let mut c = client::Client::connect(&socket).await?;
            let s: rat_proto::LlmStatusResult =
                serde_json::from_value(c.call(methods::LLM_STATUS, Value::Null).await?)?;
            println!("provider:          {}", s.provider);
            println!("critic_enabled:    {}", s.critic_enabled);
            println!("embedding_enabled: {}", s.embedding_enabled);
            println!("key openai:        {}", s.keys.openai);
            println!("key anthropic:     {}", s.keys.anthropic);
            println!("key openrouter:    {}", s.keys.openrouter);
            if let Some(err) = s.last_error {
                println!("last_error:        {}", err);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_key_files_trims_whitespace() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("open_k.txt"), "  sk-test  \n").unwrap();
        std::fs::write(tmp.path().join("antr_k.txt"), "ant-key").unwrap();
        let (keys, notes) = read_key_files(tmp.path());
        assert_eq!(keys.get("openai").map(|s| s.as_str()), Some("sk-test"));
        assert_eq!(keys.get("anthropic").map(|s| s.as_str()), Some("ant-key"));
        assert_eq!(keys.get("openrouter"), None);
        assert!(notes.is_empty());
    }

    #[test]
    fn read_key_files_missing_dir_returns_empty() {
        let (keys, notes) = read_key_files(std::path::Path::new("/nonexistent/path/xyz"));
        assert!(keys.is_empty());
        assert!(notes.is_empty());
    }

    #[test]
    fn read_key_files_empty_file_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("open_k.txt"), "   \n   ").unwrap();
        let (keys, _notes) = read_key_files(tmp.path());
        assert_eq!(keys.get("openai"), None);
    }

    #[test]
    fn read_key_files_strips_utf8_bom() {
        let tmp = tempfile::tempdir().unwrap();
        // UTF-8 BOM is 0xEF 0xBB 0xBF = '\u{feff}' in Unicode
        let bom_key = "\u{feff}sk-bom-test\n";
        std::fs::write(tmp.path().join("open_k.txt"), bom_key).unwrap();
        let (keys, notes) = read_key_files(tmp.path());
        assert_eq!(keys.get("openai").map(|s| s.as_str()), Some("sk-bom-test"),
            "BOM should be stripped before the key is stored");
        assert!(notes.is_empty());
    }

    #[test]
    fn read_key_files_skips_oversized_file() {
        let tmp = tempfile::tempdir().unwrap();
        // Write a file larger than KEY_FILE_MAX_BYTES (4096)
        let oversized: Vec<u8> = b"x".repeat(4097);
        std::fs::write(tmp.path().join("open_k.txt"), &oversized).unwrap();
        // Also write a valid anthropic key to confirm the other file still loads
        std::fs::write(tmp.path().join("antr_k.txt"), "ant-key").unwrap();
        let (keys, notes) = read_key_files(tmp.path());
        assert_eq!(keys.get("openai"), None, "oversized file must not be stored");
        assert_eq!(keys.get("anthropic").map(|s| s.as_str()), Some("ant-key"),
            "non-oversized file must still load");
        assert_eq!(notes.len(), 1, "one warning note expected for the oversized file");
        assert!(notes[0].contains("4097"), "note should mention the actual byte count");
    }
}
