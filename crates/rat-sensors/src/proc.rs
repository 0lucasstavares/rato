use std::collections::{HashMap, HashSet};

use rat_proto::NewEvent;
use serde_json::json;

/// Dev-relevant process names worth tracking (matched against /proc/<pid>/comm).
pub const ALLOWLIST: &[&str] = &[
    "cargo", "rustc", "node", "npm", "pnpm", "yarn", "python", "python3", "pytest", "go", "make",
    "cmake", "gcc", "g++", "clang", "docker", "tsc", "vitest", "jest", "claude", "codex", "aider",
    "gemini", "tmux",
];

#[derive(Debug, Clone)]
pub struct Tracked {
    pub comm: String,
    pub since_ms: i64,
}

/// Pure diff engine over successive /proc snapshots. IO lives in `scan_procs`.
#[derive(Default)]
pub struct ProcWatcher {
    tracked: HashMap<u32, Tracked>,
}

impl ProcWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Diff a fresh snapshot against the tracked set, producing
    /// proc_started / proc_exited events. Payload enrichment (cmdline/cwd)
    /// is the caller's job for started pids.
    pub fn observe(&mut self, current: &HashMap<u32, String>, now: i64) -> Vec<NewEvent> {
        let mut events = Vec::new();
        for (&pid, comm) in current {
            if !self.tracked.contains_key(&pid) {
                self.tracked.insert(pid, Tracked { comm: comm.clone(), since_ms: now });
                events.push(NewEvent {
                    kind: "proc_started".into(),
                    source: "proc".into(),
                    payload: json!({"pid": pid, "comm": comm}),
                    ..Default::default()
                });
            }
        }
        let gone: Vec<u32> = self.tracked.keys().filter(|p| !current.contains_key(p)).copied().collect();
        for pid in gone {
            let t = self.tracked.remove(&pid).expect("tracked pid");
            events.push(NewEvent {
                kind: "proc_exited".into(),
                source: "proc".into(),
                payload: json!({"pid": pid, "comm": t.comm, "duration_ms": now - t.since_ms}),
                ..Default::default()
            });
        }
        events
    }
}

/// Scan /proc for allowlisted processes: pid → comm.
pub fn scan_procs(allow: &HashSet<&str>) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return out };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else { continue };
        let comm = comm.trim();
        if allow.contains(comm) {
            out.insert(pid, comm.to_string());
        }
    }
    out
}

/// Best-effort cmdline (NUL-joined → spaces, truncated) and cwd for a pid.
pub fn proc_detail(pid: u32) -> (Option<String>, Option<String>) {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok().map(|raw| {
        let mut s = String::from_utf8_lossy(&raw).replace('\0', " ").trim().to_string();
        if s.len() > 512 {
            let mut end = 512;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
        s
    });
    let cwd = std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    (cmdline, cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(pairs: &[(u32, &str)]) -> HashMap<u32, String> {
        pairs.iter().map(|(p, c)| (*p, c.to_string())).collect()
    }

    #[test]
    fn new_pid_emits_started_then_exit_emits_exited_with_duration() {
        let mut w = ProcWatcher::new();
        let ev = w.observe(&snap(&[(42, "cargo")]), 1_000);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, "proc_started");
        assert_eq!(ev[0].payload["comm"], "cargo");

        // still running: no events
        assert!(w.observe(&snap(&[(42, "cargo")]), 2_000).is_empty());

        let ev = w.observe(&snap(&[]), 5_000);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, "proc_exited");
        assert_eq!(ev[0].payload["duration_ms"], 4_000);
    }

    #[test]
    fn scan_procs_finds_nothing_unallowed() {
        // our own test binary's comm is not in the allowlist
        let found = scan_procs(&ALLOWLIST.iter().copied().collect());
        for comm in found.values() {
            assert!(ALLOWLIST.contains(&comm.as_str()));
        }
    }
}
