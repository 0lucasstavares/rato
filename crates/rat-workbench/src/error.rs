use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkbenchError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("tmux command failed (exit {code}): {stderr}")]
    TmuxFailed { code: i32, stderr: String },

    #[error("git command failed (exit {code}): {stderr}")]
    GitFailed { code: i32, stderr: String },

    #[error("escape guard: cwd `{cwd}` is outside worktree root `{root}`")]
    EscapeGuard { cwd: String, root: String },

    #[error("canonicalize failed for `{path}`: {source}")]
    Canonicalize {
        path: String,
        source: std::io::Error,
    },

    #[error("merge not fast-forwardable: conflicts detected")]
    NeedsMergeManual,

    #[error("utf-8 decode error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("worktree directory already exists: {0}")]
    WorktreeExists(String),
}

pub type Result<T, E = WorkbenchError> = std::result::Result<T, E>;
