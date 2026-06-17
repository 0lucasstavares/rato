//! Deterministic terminal detection/classification core for M7.
//!
//! Real `/proc` and tmux scanners can feed these pure types later; this crate
//! deliberately keeps the first pass fakeable so safety and role classification
//! are testable without a live desktop session.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcEntry {
    pub pid: i64,
    pub ppid: Option<i64>,
    pub cmdline: Vec<String>,
    pub tty: Option<String>,
}

impl ProcEntry {
    pub fn executable_name(&self) -> Option<&str> {
        let argv0 = self.cmdline.first()?;
        argv0.rsplit('/').next()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TmuxPane {
    pub tty: String,
    pub target: String,
    pub socket_name: Option<String>,
    pub current_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalRole {
    Workbench,
    Foreign,
    Operator,
    Ignored,
}

impl TerminalRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Workbench => "workbench",
            Self::Foreign => "foreign",
            Self::Operator => "operator",
            Self::Ignored => "ignored",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalHit {
    pub pid: i64,
    pub tty: String,
    pub emulator: Option<String>,
    pub adapter: String,
    pub tmux_target: Option<String>,
    pub role: TerminalRole,
    pub cmd_hash: String,
}

#[derive(Debug, Clone)]
pub struct ClassifierConfig {
    pub adapter_names: HashSet<String>,
    pub ignored_hashes: HashSet<String>,
    pub operator_hashes: HashSet<String>,
    pub rato_tmux_socket: String,
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            adapter_names: ["claude", "codex", "aider", "gemini"]
                .into_iter()
                .map(String::from)
                .collect(),
            ignored_hashes: HashSet::new(),
            operator_hashes: HashSet::new(),
            rato_tmux_socket: "rato".to_string(),
        }
    }
}

pub trait ProcSource {
    fn processes(&self) -> Vec<ProcEntry>;
    fn tmux_panes(&self) -> Vec<TmuxPane>;
}

#[derive(Debug, Clone, Default)]
pub struct RealProcSource;

impl ProcSource for RealProcSource {
    fn processes(&self) -> Vec<ProcEntry> {
        read_proc_entries(Path::new("/proc"))
    }

    fn tmux_panes(&self) -> Vec<TmuxPane> {
        read_tmux_panes()
    }
}

pub fn classify<S: ProcSource>(source: &S, config: &ClassifierConfig) -> Vec<TerminalHit> {
    let processes = source.processes();
    let by_pid: HashMap<i64, ProcEntry> = processes
        .iter()
        .cloned()
        .map(|process| (process.pid, process))
        .collect();
    let panes_by_tty: HashMap<String, TmuxPane> = source
        .tmux_panes()
        .into_iter()
        .map(|pane| (pane.tty.clone(), pane))
        .collect();
    let mut hits = Vec::new();

    for process in processes {
        let Some(adapter) = process.executable_name() else {
            continue;
        };
        if !config.adapter_names.contains(adapter) {
            continue;
        }
        let Some(tty) = process.tty.clone() else {
            continue;
        };

        let cmd_hash = command_hash(&process.cmdline);
        let pane = panes_by_tty.get(&tty);
        let role = if config.ignored_hashes.contains(&cmd_hash) {
            TerminalRole::Ignored
        } else if config.operator_hashes.contains(&cmd_hash) {
            TerminalRole::Operator
        } else if pane
            .and_then(|pane| pane.socket_name.as_deref())
            .is_some_and(|socket| socket == config.rato_tmux_socket)
        {
            TerminalRole::Workbench
        } else {
            TerminalRole::Foreign
        };

        hits.push(TerminalHit {
            pid: process.pid,
            tty,
            emulator: find_emulator(&by_pid, &process),
            adapter: adapter.to_string(),
            tmux_target: pane.map(|pane| pane.target.clone()),
            role,
            cmd_hash,
        });
    }

    hits.sort_by_key(|hit| hit.pid);
    hits
}

pub fn command_hash(cmdline: &[String]) -> String {
    let mut hasher = Sha256::new();
    for arg in cmdline {
        hasher.update(arg.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        use std::fmt::Write;
        write!(out, "{byte:02x}").expect("write to String cannot fail");
    }
    out
}

fn find_emulator(by_pid: &HashMap<i64, ProcEntry>, process: &ProcEntry) -> Option<String> {
    let mut parent = process.ppid;
    while let Some(pid) = parent {
        let entry = by_pid.get(&pid)?;
        let Some(name) = entry.executable_name() else {
            parent = entry.ppid;
            continue;
        };
        if is_terminal_emulator(name) {
            return Some(name.to_string());
        }
        parent = entry.ppid;
    }
    None
}

fn is_terminal_emulator(name: &str) -> bool {
    matches!(
        name,
        "alacritty"
            | "foot"
            | "gnome-terminal-server"
            | "konsole"
            | "kitty"
            | "wezterm-gui"
            | "xterm"
    )
}

fn read_proc_entries(proc_root: &Path) -> Vec<ProcEntry> {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<i64>().ok()?;
            let dir = entry.path();
            let cmdline = read_cmdline(&dir.join("cmdline"))?;
            Some(ProcEntry {
                pid,
                ppid: read_ppid(&dir.join("stat")),
                cmdline,
                tty: read_tty(&dir.join("fd").join("0")),
            })
        })
        .collect()
}

fn read_cmdline(path: &Path) -> Option<Vec<String>> {
    let bytes = fs::read(path).ok()?;
    let args: Vec<String> = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    if args.is_empty() {
        None
    } else {
        Some(args)
    }
}

fn read_ppid(path: &Path) -> Option<i64> {
    let stat = fs::read_to_string(path).ok()?;
    parse_ppid_from_stat(&stat)
}

fn parse_ppid_from_stat(stat: &str) -> Option<i64> {
    let after_comm = stat.rsplit_once(") ")?.1;
    let mut fields = after_comm.split_whitespace();
    let _state = fields.next()?;
    fields.next()?.parse().ok()
}

fn read_tty(fd0: &Path) -> Option<String> {
    let target = fs::read_link(fd0).ok()?;
    let text = target.to_string_lossy();
    if text.starts_with("/dev/pts/") || text.starts_with("/dev/tty") {
        Some(text.into_owned())
    } else {
        None
    }
}

fn read_tmux_panes() -> Vec<TmuxPane> {
    let Ok(output) = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_tty}\t#{session_name}:#{window_index}.#{pane_index}\t#{socket_path}\t#{pane_current_command}",
        ])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_tmux_pane_line)
        .collect()
}

fn parse_tmux_pane_line(line: &str) -> Option<TmuxPane> {
    let mut fields = line.split('\t');
    let tty = fields.next()?.trim();
    let target = fields.next()?.trim();
    let socket_path = fields.next().map(str::trim).filter(|s| !s.is_empty());
    let current_command = fields.next().map(str::trim).filter(|s| !s.is_empty());
    if tty.is_empty() || target.is_empty() {
        return None;
    }
    Some(TmuxPane {
        tty: tty.to_string(),
        target: target.to_string(),
        socket_name: socket_path
            .and_then(|path| Path::new(path).file_name())
            .map(|name| name.to_string_lossy().into_owned()),
        current_command: current_command.map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeSource {
        processes: Vec<ProcEntry>,
        panes: Vec<TmuxPane>,
    }

    impl ProcSource for FakeSource {
        fn processes(&self) -> Vec<ProcEntry> {
            self.processes.clone()
        }

        fn tmux_panes(&self) -> Vec<TmuxPane> {
            self.panes.clone()
        }
    }

    fn proc(pid: i64, ppid: Option<i64>, argv0: &str, tty: Option<&str>) -> ProcEntry {
        ProcEntry {
            pid,
            ppid,
            cmdline: vec![argv0.to_string()],
            tty: tty.map(String::from),
        }
    }

    #[test]
    fn classifies_rato_tmux_pane_as_workbench() {
        let source = FakeSource {
            processes: vec![
                proc(10, None, "tmux", None),
                proc(11, Some(10), "claude", Some("/dev/pts/4")),
            ],
            panes: vec![TmuxPane {
                tty: "/dev/pts/4".into(),
                target: "rato-task:0.0".into(),
                socket_name: Some("rato".into()),
                current_command: Some("claude".into()),
            }],
        };

        let hits = classify(&source, &ClassifierConfig::default());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, TerminalRole::Workbench);
        assert_eq!(hits[0].tmux_target.as_deref(), Some("rato-task:0.0"));
    }

    #[test]
    fn classifies_foreign_llm_terminal_and_emulator() {
        let source = FakeSource {
            processes: vec![
                proc(20, None, "kitty", None),
                proc(21, Some(20), "bash", Some("/dev/pts/8")),
                proc(
                    22,
                    Some(21),
                    "/home/me/.local/bin/codex",
                    Some("/dev/pts/8"),
                ),
            ],
            panes: vec![],
        };

        let hits = classify(&source, &ClassifierConfig::default());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].adapter, "codex");
        assert_eq!(hits[0].role, TerminalRole::Foreign);
        assert_eq!(hits[0].emulator.as_deref(), Some("kitty"));
    }

    #[test]
    fn remembers_ignored_and_operator_command_hashes() {
        let ignored = vec![
            "claude".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        let operator = vec!["gemini".to_string()];
        let mut config = ClassifierConfig::default();
        config.ignored_hashes.insert(command_hash(&ignored));
        config.operator_hashes.insert(command_hash(&operator));
        let source = FakeSource {
            processes: vec![
                ProcEntry {
                    pid: 30,
                    ppid: None,
                    cmdline: ignored,
                    tty: Some("/dev/pts/10".into()),
                },
                ProcEntry {
                    pid: 31,
                    ppid: None,
                    cmdline: operator,
                    tty: Some("/dev/pts/11".into()),
                },
            ],
            panes: vec![],
        };

        let hits = classify(&source, &config);
        assert_eq!(hits[0].role, TerminalRole::Ignored);
        assert_eq!(hits[1].role, TerminalRole::Operator);
    }

    #[test]
    fn skips_non_agent_and_missing_tty_processes() {
        let source = FakeSource {
            processes: vec![
                proc(40, None, "bash", Some("/dev/pts/1")),
                proc(41, None, "aider", None),
            ],
            panes: vec![],
        };

        assert!(classify(&source, &ClassifierConfig::default()).is_empty());
    }

    #[test]
    fn parses_proc_stat_ppid_with_spaces_in_command() {
        assert_eq!(
            parse_ppid_from_stat("123 (weird command) S 77 1 2 3"),
            Some(77)
        );
    }

    #[test]
    fn parses_tmux_pane_line_with_socket_basename() {
        let pane =
            parse_tmux_pane_line("/dev/pts/3\trun:0.1\t/tmp/tmux-1000/rato\tclaude").unwrap();
        assert_eq!(pane.tty, "/dev/pts/3");
        assert_eq!(pane.target, "run:0.1");
        assert_eq!(pane.socket_name.as_deref(), Some("rato"));
        assert_eq!(pane.current_command.as_deref(), Some("claude"));
    }
}
