use std::path::Path;
use std::process::Command;

use crate::error::{Result, WorkbenchError};

/// A handle to a tmux server isolated by a named socket (`-L <socket_name>`).
///
/// Production code uses `socket_name = "rato"`.
/// Tests use `socket_name = format!("rato-test-{}", std::process::id())` to
/// keep each test run in its own server.
///
/// # Security invariant
/// All operations shell out via `std::process::Command` with separate argv
/// items. We NEVER construct `sh -c "... interpolated ..."` strings.
#[derive(Debug, Clone)]
pub struct Tmux {
    pub socket_name: String,
}

/// Identifies a specific window inside a session, as `"session:window"`.
pub type Target = String;

impl Tmux {
    pub fn new(socket_name: impl Into<String>) -> Self {
        Self {
            socket_name: socket_name.into(),
        }
    }

    /// Base `tmux -L <socket>` command with the given subcommand args appended.
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new("tmux");
        c.arg("-L").arg(&self.socket_name);
        for a in args {
            c.arg(a);
        }
        c
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let out = self.cmd(args).output()?;
        if out.status.success() {
            Ok(String::from_utf8(out.stdout)?)
        } else {
            Err(WorkbenchError::TmuxFailed {
                code: out.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            })
        }
    }

    /// Start the tmux server if it is not already running.
    ///
    /// We use `start-server` which is a no-op when the server is already up.
    pub fn ensure_server(&self) -> Result<()> {
        self.run(&["start-server"])?;
        Ok(())
    }

    /// Create a session named `name` if it does not already exist.
    pub fn ensure_session(&self, name: &str) -> Result<()> {
        // `has-session` returns exit 0 if session exists, non-zero otherwise.
        let exists = self
            .cmd(&["has-session", "-t", name])
            .output()?
            .status
            .success();
        if !exists {
            // `new-session -d` creates a detached session.
            self.run(&["new-session", "-d", "-s", name])?;
        }
        Ok(())
    }

    /// Create a new window inside `session`, set its working directory to
    /// `cwd`, and return the target string `"session:window"`.
    pub fn new_window(&self, session: &str, name: &str, cwd: &Path) -> Result<Target> {
        let cwd_str = cwd
            .to_str()
            .ok_or_else(|| WorkbenchError::Parse("cwd path is not valid UTF-8".into()))?;
        self.run(&[
            "new-window",
            "-d",
            "-t",
            session,
            "-n",
            name,
            "-c",
            cwd_str,
        ])?;
        Ok(format!("{}:{}", session, name))
    }

    /// Send `cmd` as literal keystrokes to `target`, then send Enter.
    ///
    /// Uses `-l` (literal flag) so tmux does not interpret special characters.
    pub fn run_in_window(&self, target: &str, cmd: &str) -> Result<()> {
        // send-keys -l <target> <cmd>
        self.run(&["send-keys", "-l", "-t", target, cmd])?;
        // send Enter as a separate keystroke
        self.run(&["send-keys", "-t", target, "Enter"])?;
        Ok(())
    }

    /// Capture the last `lines` lines of the pane in `target` and return them
    /// as a String.
    pub fn capture_tail(&self, target: &str, lines: u32) -> Result<String> {
        let start = format!("-{}", lines);
        let out = self.run(&[
            "capture-pane",
            "-p",          // print to stdout
            "-t",
            target,
            "-S",
            &start, // start N lines from bottom
        ])?;
        Ok(out)
    }

    /// Return `true` if the window identified by `target` still exists.
    pub fn window_alive(&self, target: &str) -> bool {
        self.cmd(&["has-session", "-t", target])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Kill a single window.
    pub fn kill_window(&self, target: &str) -> Result<()> {
        self.run(&["kill-window", "-t", target])?;
        Ok(())
    }

    /// Kill the entire tmux server (all sessions).
    pub fn kill_server(&self) -> Result<()> {
        // Tolerate failure — server may already be gone.
        let _ = self.cmd(&["kill-server"]).output();
        Ok(())
    }
}
