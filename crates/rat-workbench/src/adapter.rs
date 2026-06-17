//! Agent adapter trait and built-in implementations.
//!
//! # Architecture §6
//! Each adapter wraps a concrete agent CLI (or a fake for tests). It is
//! responsible for:
//!   1. Detecting whether the binary is available on PATH.
//!   2. Returning the shell command to run inside a tmux window.
//!   3. (M7) Parsing agent transcripts and locating transcript directories.
//!      Full parsing is deferred to M7 terminal work; current stubs return empty.

use std::path::Path;
use std::process::Command;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A pluggable agent adapter (§6).
///
/// Adapters are `Send + Sync` so they can be shared across async tasks.
pub trait AgentAdapter: Send + Sync {
    /// Short, stable name used in `agent_runs.adapter`.
    fn name(&self) -> &str;

    /// Return `true` if the agent's CLI binary is reachable on `PATH`.
    fn detect_binary(&self) -> bool;

    /// Return the shell command that runs the agent headlessly inside a tmux
    /// window. The caller wraps this in `cd <worktree> && <cmd>; echo RATO_EXIT=$?`.
    fn headless_cmd(&self, task: &str, worktree: &Path) -> String;

    /// Return the shell command to run inside a container with the worktree
    /// mounted at `/workspace`. By default this reuses the headless command
    /// with `/workspace` as the worktree path; adapters can override when their
    /// host paths are not meaningful inside containers.
    fn container_cmd(&self, task: &str) -> String {
        self.headless_cmd(task, Path::new("/workspace"))
    }

    /// Parse an agent transcript file and return a summary string.
    /// Adapters parse their native JSONL transcript format into plain text
    /// suitable for `agent_output` observations.
    fn parse_transcript(&self, _transcript_path: &Path) -> Option<String> {
        None
    }

    /// Return the directories where this agent stores transcript files, rooted
    /// at `worktree`.
    ///
    /// Return an empty `Vec` when the adapter has no known transcript location.
    fn transcript_dirs(&self, _worktree: &Path) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    /// Quick health check: returns `Ok(())` when the adapter appears usable.
    ///
    /// Default implementation checks `detect_binary`. Real adapters may do
    /// more (e.g. verify the CLI can print a version string).
    fn health(&self) -> Result<(), String> {
        if self.detect_binary() {
            Ok(())
        } else {
            Err(format!("binary '{}' not found on PATH", self.name()))
        }
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn binary_on_path(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn parse_jsonl_transcript(path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let mut lines = Vec::new();
    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        collect_text_fields(&value, &mut lines);
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn collect_text_fields(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|value| value.as_str()) {
                push_text(out, text);
            } else if let Some(content) = map.get("content").and_then(|value| value.as_str()) {
                push_text(out, content);
            }
            for child in map.values() {
                collect_text_fields(child, out);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_text_fields(child, out);
            }
        }
        _ => {}
    }
}

fn push_text(out: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() && out.last().is_none_or(|last| last != trimmed) {
        out.push(trimmed.to_string());
    }
}

// ---------------------------------------------------------------------------
// FakeAgent
// ---------------------------------------------------------------------------

/// A deterministic fake agent for integration tests.
///
/// Runs `bash <fixture>/tests/fixtures/fakeagent.sh` inside the worktree.
/// The fixture path is the workspace root of the current crate
/// (resolved at build time via `CARGO_MANIFEST_DIR`).
pub struct FakeAgent {
    /// Absolute path to `fakeagent.sh`.
    fixture_path: std::path::PathBuf,
}

impl FakeAgent {
    /// Construct using the standard fixture location relative to `repo_root`.
    ///
    /// `repo_root` should be the path to the `rat-workbench` crate (or any
    /// ancestor that contains `tests/fixtures/fakeagent.sh`).
    pub fn new(repo_root: &Path) -> Self {
        Self {
            fixture_path: repo_root.join("tests/fixtures/fakeagent.sh"),
        }
    }

    /// Convenience constructor that uses the compile-time manifest directory.
    ///
    /// This is the right constructor for in-crate integration tests.
    pub fn from_manifest() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        Self::new(root)
    }
}

impl AgentAdapter for FakeAgent {
    fn name(&self) -> &str {
        "fakeagent"
    }

    fn detect_binary(&self) -> bool {
        self.fixture_path.exists()
    }

    fn headless_cmd(&self, _task: &str, _worktree: &Path) -> String {
        format!("bash {}", self.fixture_path.display())
    }

    fn container_cmd(&self, _task: &str) -> String {
        "echo fakeagent fixture is host-only; \
         use a docker image with the desired agent installed; exit 2"
            .to_string()
    }

    fn health(&self) -> Result<(), String> {
        if self.fixture_path.exists() {
            Ok(())
        } else {
            Err(format!(
                "fakeagent fixture not found: {}",
                self.fixture_path.display()
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// ClaudeCode
// ---------------------------------------------------------------------------

/// Adapter for the `claude` CLI (Claude Code).
///
/// Requires the `claude` binary on PATH.
/// `headless_cmd` runs Claude in headless / non-interactive mode.
pub struct ClaudeCode;

impl AgentAdapter for ClaudeCode {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn detect_binary(&self) -> bool {
        binary_on_path("claude")
    }

    fn headless_cmd(&self, task: &str, _worktree: &Path) -> String {
        // --dangerously-skip-permissions=false is intentional: we want the
        // full permission model to apply even in headless mode.
        format!(
            "claude -p {} --output-format json --dangerously-skip-permissions=false",
            shell_quote(task)
        )
    }

    fn parse_transcript(&self, transcript_path: &Path) -> Option<String> {
        parse_jsonl_transcript(transcript_path)
    }

    fn transcript_dirs(&self, worktree: &Path) -> Vec<std::path::PathBuf> {
        vec![worktree.join(".claude/projects")]
    }
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

/// Adapter for the `codex` CLI (OpenAI Codex agent).
///
/// Requires the `codex` binary on PATH.
pub struct Codex;

impl AgentAdapter for Codex {
    fn name(&self) -> &str {
        "codex"
    }

    fn detect_binary(&self) -> bool {
        binary_on_path("codex")
    }

    fn headless_cmd(&self, task: &str, _worktree: &Path) -> String {
        format!("codex exec {}", shell_quote(task))
    }

    fn parse_transcript(&self, transcript_path: &Path) -> Option<String> {
        parse_jsonl_transcript(transcript_path)
    }

    fn transcript_dirs(&self, worktree: &Path) -> Vec<std::path::PathBuf> {
        vec![worktree.join(".codex/sessions")]
    }
}

// ---------------------------------------------------------------------------
// Shell quoting helper
// ---------------------------------------------------------------------------

/// Minimal single-quote shell escaping for a literal string argument.
///
/// Single-quotes in the task text are escaped as `'\''`.
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', r"'\''");
    format!("'{}'", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_agent_name() {
        let fa = FakeAgent::from_manifest();
        assert_eq!(fa.name(), "fakeagent");
    }

    #[test]
    fn fake_agent_fixture_exists() {
        let fa = FakeAgent::from_manifest();
        assert!(fa.detect_binary(), "fakeagent fixture should exist");
    }

    #[test]
    fn fake_agent_headless_cmd_contains_sh() {
        let fa = FakeAgent::from_manifest();
        let cmd = fa.headless_cmd("do something", Path::new("/tmp/wt"));
        assert!(
            cmd.contains("fakeagent.sh"),
            "cmd should reference fakeagent.sh, got: {cmd}"
        );
    }

    #[test]
    fn claude_code_name() {
        assert_eq!(ClaudeCode.name(), "claude-code");
    }

    #[test]
    fn claude_code_headless_cmd_shape() {
        let cmd = ClaudeCode.headless_cmd("fix tests", Path::new("/tmp/wt"));
        assert!(
            cmd.contains("claude -p"),
            "should invoke claude, got: {cmd}"
        );
        assert!(
            cmd.contains("--output-format json"),
            "should set json output, got: {cmd}"
        );
        assert!(
            cmd.contains("fix tests"),
            "should embed task text, got: {cmd}"
        );
    }

    #[test]
    fn codex_name() {
        assert_eq!(Codex.name(), "codex");
    }

    #[test]
    fn codex_headless_cmd_shape() {
        let cmd = Codex.headless_cmd("refactor", Path::new("/tmp/wt"));
        assert!(
            cmd.starts_with("codex exec"),
            "should start with 'codex exec', got: {cmd}"
        );
        assert!(
            cmd.contains("refactor"),
            "should embed task text, got: {cmd}"
        );
    }

    #[test]
    fn claude_transcript_dirs_are_project_local() {
        let dirs = ClaudeCode.transcript_dirs(Path::new("/repo"));
        assert_eq!(
            dirs,
            vec![Path::new("/repo/.claude/projects").to_path_buf()]
        );
    }

    #[test]
    fn codex_transcript_dirs_are_project_local() {
        let dirs = Codex.transcript_dirs(Path::new("/repo"));
        assert_eq!(dirs, vec![Path::new("/repo/.codex/sessions").to_path_buf()]);
    }

    #[test]
    fn claude_jsonl_transcript_extracts_nested_text() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("claude.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"fix tests"}]}}"#
                .to_string()
                + "\n"
                + r#"{"type":"assistant","message":{"content":[{"type":"text","text":"updated parser"}]}}"#
                + "\n",
        )
        .unwrap();

        let parsed = ClaudeCode.parse_transcript(&path).unwrap();
        assert!(parsed.contains("fix tests"));
        assert!(parsed.contains("updated parser"));
    }

    #[test]
    fn codex_jsonl_transcript_extracts_content_and_text() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("codex.jsonl");
        std::fs::write(
            &path,
            r#"{"role":"user","content":"refactor"}"#.to_string()
                + "\n"
                + r#"{"event":"message","text":"done"}"#
                + "\n",
        )
        .unwrap();

        let parsed = Codex.parse_transcript(&path).unwrap();
        assert_eq!(parsed, "refactor\ndone");
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        let q = shell_quote("it's a task");
        assert_eq!(q, r"'it'\''s a task'");
    }

    #[test]
    fn shell_quote_plain() {
        let q = shell_quote("plain task");
        assert_eq!(q, "'plain task'");
    }

    #[test]
    fn fake_agent_container_cmd_is_explicitly_host_only() {
        let fa = FakeAgent::from_manifest();
        let cmd = fa.container_cmd("task");
        assert!(cmd.contains("host-only"));
    }
}
