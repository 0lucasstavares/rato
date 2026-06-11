pub mod error;
pub mod tmux;
pub mod worktree;

pub use error::{Result, WorkbenchError};
pub use tmux::{Target, Tmux};
pub use worktree::{
    commits_ahead, create, diffstat, escape_guard, full_diff, is_fast_mergeable,
    merge_into_live, remove, repo_hash, scrub_agent_env, MergeOutcome, Worktree,
};
