//! TaskRunner: orchestrates agent runs through the worktree + tmux + store pipeline.
//!
//! # Responsibilities
//! - `start` — insert an `AgentRun` record, create a worktree, open a tmux
//!   window, and run the adapter command (wrapped so the exit code is visible
//!   in the captured output as `RATO_EXIT=<n>`).
//! - `poll` — capture the tmux tail; detect `RATO_EXIT=<n>` to transition the
//!   run to `done` or `failed`; also `failed` if the window is gone.
//! - `merge_back` — compute diffstat + full diff; large diffs (>32 KB) are
//!   stored as blobs; insert an R2 `Approval` (expires 60 min from now).
//! - `execute_merge` — re-verify the approval is `approved` and not expired,
//!   confirm the branch is fast-mergeable, run the merge, write execution
//!   metadata onto the approval, and set the run status to `merged`.
//! - `deny` — record the decision on the approval; the live repo is never
//!   touched (M4 hard invariant).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use regex::Regex;
use serde_json::json;

use rat_core::clock::Clock;
use rat_policy::{risk_tier, ActionKind, RiskOutcome};
use rat_store::{
    rows::{AgentRun, Approval, NewAgentRun, NewApproval},
    store::Store,
};

use crate::adapter::AgentAdapter;
use crate::error::{Result, WorkbenchError};
use crate::tmux::Tmux;
use crate::worktree::{self, MergeOutcome, Worktree};

/// 32 KB threshold for inlining diffs vs storing them as blobs.
const DIFF_INLINE_LIMIT: usize = 32 * 1024;

/// Approval expiry window in milliseconds (60 minutes).
const APPROVAL_EXPIRY_MS: i64 = 60 * 60 * 1000;

// ---------------------------------------------------------------------------
// TaskRunner
// ---------------------------------------------------------------------------

/// Owns the tmux handle + store reference needed to drive agent runs.
#[derive(Clone)]
pub struct TaskRunner {
    pub store: Store,
    pub tmux: Tmux,
    pub clock: Arc<dyn Clock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionBackend {
    Local,
    Docker { image: String },
}

impl TaskRunner {
    pub fn new(store: Store, tmux: Tmux, clock: Arc<dyn Clock>) -> Self {
        Self { store, tmux, clock }
    }

    // -----------------------------------------------------------------------
    // start
    // -----------------------------------------------------------------------

    /// Insert an `AgentRun`, create a worktree, open a tmux window, and run
    /// the adapter command.
    ///
    /// The command sent to the tmux window is:
    ///   `cd <worktree> && <adapter_cmd>; echo RATO_EXIT=$?`
    ///
    /// This ensures the shell cwd is the worktree AND the exit code is visible
    /// in `capture_tail` output so `poll` can detect completion.
    ///
    /// The `tmux_target` is stored in the `AgentRun` record in the store so
    /// that `poll` can retrieve it from the store without any in-memory state.
    pub async fn start(
        &self,
        project_root: &Path,
        project_id: &str,
        title: &str,
        adapter: &dyn AgentAdapter,
        base: &str,
    ) -> Result<AgentRun> {
        self.start_with_backend(
            project_root,
            project_id,
            title,
            adapter,
            base,
            ExecutionBackend::Local,
        )
        .await
    }

    pub async fn start_with_backend(
        &self,
        project_root: &Path,
        project_id: &str,
        title: &str,
        adapter: &dyn AgentAdapter,
        base: &str,
        backend: ExecutionBackend,
    ) -> Result<AgentRun> {
        let now = self.clock.now_ms();

        // Derive a URL-safe slug from the title (lowercase, replace non-alnum with '-').
        let slug = title_to_slug(title);
        // Use the current timestamp (as hex) to get a unique task_id.
        let task_id = format!("{:x}", now);

        // Create the worktree (can fail with git errors).
        let worktree = worktree::create(project_root, &task_id, &slug, base)?;

        // Boot tmux and open a window BEFORE inserting the run so we can
        // store the tmux_target in the initial record.
        self.tmux.ensure_server()?;
        let session = "rato";
        self.tmux.ensure_session(session)?;

        // Use a unique window name derived from slug + task_id (first 8 hex chars).
        let window_name = format!(
            "{}-{}",
            &slug[..slug.len().min(12)],
            &task_id[..task_id.len().min(8)]
        );
        let target = self
            .tmux
            .new_window(session, &window_name, &worktree.path)?;

        // Register the run in the store with the tmux_target already set.
        let run = self
            .store
            .insert_agent_run(NewAgentRun {
                adapter: adapter.name().to_string(),
                task_title: title.to_string(),
                project_id: project_id.to_string(),
                worktree_path: worktree.path.display().to_string(),
                branch: worktree.branch.clone(),
                tmux_target: Some(target.clone()),
                mode: backend.mode_label(),
                tokens: json!({}),
                cost_usd: 0.0,
                started: now,
            })
            .await
            .map_err(|e| WorkbenchError::Parse(format!("store error: {e}")))?;

        // Build the wrapped command: cd into worktree, run agent cmd, then
        // echo the exit code so poll() can detect completion.
        let wrapped = wrapped_run_command(title, adapter, &worktree.path, &backend);
        self.tmux.run_in_window(&target, &wrapped)?;

        Ok(run)
    }

    // -----------------------------------------------------------------------
    // poll
    // -----------------------------------------------------------------------

    /// Capture the tail of the tmux window for `run` and update its status if
    /// a `RATO_EXIT=<n>` marker is found.
    ///
    /// Returns the updated `AgentRun` (fetched from the store after any
    /// status change).
    pub async fn poll(&self, run_id: &str) -> Result<Option<AgentRun>> {
        let run = self
            .store
            .get_agent_run(run_id.to_string())
            .await
            .map_err(|e| WorkbenchError::Parse(format!("store error: {e}")))?;

        let run = match run {
            Some(r) => r,
            None => return Ok(None),
        };

        // Already terminal — nothing to do.
        if run.status != "running" {
            return Ok(Some(run));
        }

        let target = match &run.tmux_target {
            Some(t) => t.clone(),
            None => {
                // No tmux target means we can't poll — leave as running.
                return Ok(Some(run));
            }
        };

        let re = Regex::new(r"RATO_EXIT=(\d+)").expect("valid regex");

        let (new_status, should_update) = if self.tmux.window_alive(&target) {
            // Try to capture the last 50 lines.
            match self.tmux.capture_tail(&target, 50) {
                Ok(output) => {
                    if let Some(caps) = re.captures(&output) {
                        let code: i32 = caps[1].parse().unwrap_or(1);
                        if code == 0 {
                            ("done", true)
                        } else {
                            ("failed", true)
                        }
                    } else {
                        // Still running.
                        ("running", false)
                    }
                }
                Err(_) => ("failed", true),
            }
        } else {
            // Window is gone but we never saw RATO_EXIT — treat as failed.
            ("failed", true)
        };

        if should_update {
            let now = self.clock.now_ms();
            self.store
                .update_agent_run_status(
                    run.id.clone(),
                    new_status.to_string(),
                    Some(now),
                    None,
                    None,
                )
                .await
                .map_err(|e| WorkbenchError::Parse(format!("store update error: {e}")))?;
        }

        // Re-fetch to return the freshest state.
        let updated = self
            .store
            .get_agent_run(run_id.to_string())
            .await
            .map_err(|e| WorkbenchError::Parse(format!("store error: {e}")))?;
        Ok(updated)
    }

    // -----------------------------------------------------------------------
    // merge_back
    // -----------------------------------------------------------------------

    /// Compute the diff between the worktree branch and its base, create an
    /// `Approval` record (R2, expires 60 min), and return it.
    ///
    /// If the diff exceeds 32 KB it is stored as a blob and the payload
    /// references the blob id instead of inlining the diff text.
    pub async fn merge_back(&self, run_id: &str) -> Result<Approval> {
        let run = self
            .store
            .get_agent_run(run_id.to_string())
            .await
            .map_err(|e| WorkbenchError::Parse(format!("store error: {e}")))?
            .ok_or_else(|| WorkbenchError::Parse(format!("run {run_id} not found")))?;

        let worktree_path = PathBuf::from(&run.worktree_path);
        let wt = Worktree {
            path: worktree_path.clone(),
            branch: run.branch.clone(),
            // We need the base SHA. Use worktree's git log to find it.
            // Reconstruct the base by finding merge-base with HEAD of main.
            // Since we stored base_sha in Worktree during create() but not in
            // the DB, we derive it here by looking at the branch's root commit.
            base: derive_base_sha(&worktree_path, &run.branch)?,
        };

        // Compute diffstat and full diff.
        let stat_str = worktree::diffstat(&wt)?;
        let diff_str = worktree::full_diff(&wt)?;

        // Policy check: MergeBack is R2.
        let tier = match risk_tier(ActionKind::MergeBack) {
            RiskOutcome::Tier(t) => t,
            RiskOutcome::Refused => unreachable!("MergeBack is R2, not Refused"),
        };
        let risk_num = tier_to_i64(tier);

        let now = self.clock.now_ms();
        let expires_at = now + APPROVAL_EXPIRY_MS;

        // Inline or blob the diff.
        let diff_payload = if diff_str.len() > DIFF_INLINE_LIMIT {
            let blob = self
                .store
                .insert_blob(diff_str.into_bytes(), now)
                .await
                .map_err(|e| WorkbenchError::Parse(format!("blob insert error: {e}")))?;
            json!({ "blob_id": blob.id })
        } else {
            json!({ "inline": diff_str })
        };

        // Derive the project root from the worktree path. The worktree lives
        // under `~/.local/share/rato/worktrees/<hash>/<task_id>/`, so we can't
        // trivially reverse it. Instead, walk up until we find a `.git` file
        // (not directory — worktrees have a `.git` file pointing back to main).
        // Then read the main repo path from it.
        let project_root = find_project_root_from_worktree(&worktree_path)?;

        let payload = json!({
            "branch": run.branch,
            "target": project_root.display().to_string(),
            "diffstat": stat_str,
            "diff": diff_payload,
        });

        let approval = self
            .store
            .insert_approval(NewApproval {
                kind: "merge_back".to_string(),
                risk: risk_num,
                title: format!("Merge back: {}", run.task_title),
                reason: format!(
                    "Agent run '{}' (adapter: {}) completed; merge worktree branch {} into live repo.",
                    run.task_title, run.adapter, run.branch
                ),
                cwd: Some(worktree_path.display().to_string()),
                target: Some(project_root.display().to_string()),
                agent_identity: run.adapter.clone(),
                payload,
                expected_impact: json!({ "diffstat": stat_str }),
                expires_at,
            })
            .await
            .map_err(|e| WorkbenchError::Parse(format!("approval insert error: {e}")))?;

        Ok(approval)
    }

    // -----------------------------------------------------------------------
    // execute_merge
    // -----------------------------------------------------------------------

    /// Execute the merge for an approved `Approval`.
    ///
    /// Pre-conditions checked at runtime:
    /// - `approval.status == "approved"`
    /// - `approval.expires_at > now`
    /// - The branch is fast-mergeable.
    ///
    /// On success:
    /// - The live repo HEAD moves.
    /// - `set_approval_execution` writes the execution result.
    /// - The associated `AgentRun` is set to `merged`.
    pub async fn execute_merge(&self, approval: &Approval) -> Result<MergeOutcome> {
        let now = self.clock.now_ms();

        if approval.status != "approved" {
            return Err(WorkbenchError::Parse(format!(
                "approval {} is not approved (status={})",
                approval.id, approval.status
            )));
        }
        if approval.expires_at <= now {
            return Err(WorkbenchError::Parse(format!(
                "approval {} has expired (expires_at={}, now={now})",
                approval.id, approval.expires_at
            )));
        }

        // Extract branch and target from payload.
        let branch = approval
            .payload
            .get("branch")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WorkbenchError::Parse("approval payload missing 'branch'".into()))?
            .to_string();

        let target = approval
            .payload
            .get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WorkbenchError::Parse("approval payload missing 'target'".into()))?
            .to_string();

        let project_root = PathBuf::from(&target);

        // Check fast-mergeability.
        if !worktree::is_fast_mergeable(&project_root, &branch)? {
            return Ok(MergeOutcome::NeedsManual);
        }

        // Execute the merge.
        let outcome = worktree::merge_into_live(&project_root, &branch)?;

        let commit_sha = match &outcome {
            MergeOutcome::Merged { commit_sha } => commit_sha.clone(),
            MergeOutcome::NeedsManual => {
                return Ok(MergeOutcome::NeedsManual);
            }
        };

        let ended = self.clock.now_ms();

        // Write execution metadata onto the approval.
        self.store
            .set_approval_execution(
                approval.id.clone(),
                json!({
                    "started": now,
                    "ended": ended,
                    "exit_code": 0,
                    "commit_sha": commit_sha,
                    "verified_target": target,
                }),
            )
            .await
            .map_err(|e| WorkbenchError::Parse(format!("set_approval_execution error: {e}")))?;

        // Find the associated run and mark it merged.
        // We locate the run by matching the branch in recent runs.
        if let Some(run) = find_run_by_branch(&self.store, &branch).await? {
            self.store
                .update_agent_run_status(
                    run.id,
                    "merged".to_string(),
                    Some(ended),
                    Some(format!("Merged at commit {}", &commit_sha[..8])),
                    None,
                )
                .await
                .map_err(|e| WorkbenchError::Parse(format!("update run status error: {e}")))?;
        }

        Ok(outcome)
    }

    // -----------------------------------------------------------------------
    // deny
    // -----------------------------------------------------------------------

    /// Record an operator denial for an approval.
    ///
    /// The live repository is **never touched** — this is the M4 hard
    /// invariant. Only the approval record's status changes.
    pub async fn deny(&self, approval_id: &str, note: Option<&str>) -> Result<Approval> {
        let now = self.clock.now_ms();
        let decided = self
            .store
            .decide_approval(
                approval_id.to_string(),
                "denied".to_string(),
                now,
                "cli".to_string(),
                note.map(|s| s.to_string()),
            )
            .await
            .map_err(|e| WorkbenchError::Parse(format!("decide_approval error: {e}")))?;
        Ok(decided)
    }
}

impl ExecutionBackend {
    fn mode_label(&self) -> String {
        match self {
            ExecutionBackend::Local => "headless".to_string(),
            ExecutionBackend::Docker { image } => format!("docker:{image}"),
        }
    }
}

pub fn docker_run_command(image: &str, worktree_path: &Path, inner_cmd: &str) -> String {
    format!(
        "docker run --rm -i -v {}:/workspace -w /workspace {} sh -lc {}",
        shell_escape_path(worktree_path),
        shell_quote(image),
        shell_quote(inner_cmd),
    )
}

fn wrapped_run_command(
    title: &str,
    adapter: &dyn AgentAdapter,
    worktree_path: &Path,
    backend: &ExecutionBackend,
) -> String {
    match backend {
        ExecutionBackend::Local => {
            let adapter_cmd = adapter.headless_cmd(title, worktree_path);
            format!(
                "cd {} && {}; echo RATO_EXIT=$?",
                shell_escape_path(worktree_path),
                adapter_cmd
            )
        }
        ExecutionBackend::Docker { image } => {
            let inner_cmd = adapter.container_cmd(title);
            format!(
                "{}; echo RATO_EXIT=$?",
                docker_run_command(image, worktree_path, &inner_cmd)
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Convert a run title into a URL-safe slug (lowercase, non-alnum → '-',
/// runs of '-' collapsed, leading/trailing '-' stripped).
fn title_to_slug(title: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true; // suppress leading dashes
    for c in title.chars() {
        if c.is_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    // Trim trailing dash
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "task".to_string()
    } else {
        slug
    }
}

/// Produce a single-quoted shell argument for a path, escaping single-quotes.
fn shell_escape_path(path: &Path) -> String {
    let s = path.display().to_string();
    let escaped = s.replace('\'', r"'\''");
    format!("'{}'", escaped)
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', r"'\''");
    format!("'{}'", escaped)
}

/// Map policy `Tier` to the integer stored in `approvals.risk`.
fn tier_to_i64(tier: rat_policy::Tier) -> i64 {
    match tier {
        rat_policy::Tier::R0 => 0,
        rat_policy::Tier::R1 => 1,
        rat_policy::Tier::R2 => 2,
        rat_policy::Tier::R3 => 3,
    }
}

/// Derive the base SHA for a worktree by reading the first-parent commit
/// before the branch diverged.
///
/// Strategy: run `git log --oneline <branch>` inside the worktree and find
/// the merge-base with the parent repo's HEAD. Since we don't have the
/// original `base` stored in the DB, we use `git merge-base HEAD~N ...` with
/// `N = commits_ahead`. If `commits_ahead` is 0 we fall back to `HEAD`.
fn derive_base_sha(worktree_path: &Path, _branch: &str) -> Result<String> {
    use std::process::Command;
    // Find main repo root via the .git file pointer.
    let main_repo = find_project_root_from_worktree(worktree_path)?;

    // commits_ahead from the worktree — we need the base. Use `rev-parse HEAD`
    // on the main repo as the base candidate (the point we branched from).
    // More precisely: the base is the common ancestor of the worktree HEAD and
    // the main repo HEAD, which is just the commit the branch was cut from.
    let out = Command::new("git")
        .current_dir(&main_repo)
        .args(["rev-parse", "HEAD"])
        .output()?;
    if !out.status.success() {
        return Err(WorkbenchError::GitFailed {
            code: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8(out.stdout)
        .map_err(WorkbenchError::Utf8)?
        .trim()
        .to_string())
}

/// Walk up from `worktree_path` to find the main repo root.
///
/// A git worktree has a `.git` *file* (not directory) containing:
///   `gitdir: /path/to/main/.git/worktrees/<name>`
///
/// We read it and strip the trailing 3 path components.
fn find_project_root_from_worktree(worktree_path: &Path) -> Result<PathBuf> {
    let git_file = worktree_path.join(".git");
    if !git_file.is_file() {
        // Maybe the path IS the main repo (in tests with non-worktree repos).
        if worktree_path.join(".git").is_dir() {
            return Ok(worktree_path.to_path_buf());
        }
        // Walk up to find a .git directory.
        let mut cur = worktree_path.to_path_buf();
        loop {
            if cur.join(".git").exists() {
                return Ok(cur);
            }
            match cur.parent() {
                Some(p) => cur = p.to_path_buf(),
                None => break,
            }
        }
        return Err(WorkbenchError::Parse(format!(
            "could not find .git from {}",
            worktree_path.display()
        )));
    }

    let content = std::fs::read_to_string(&git_file).map_err(WorkbenchError::Io)?;
    let gitdir = content
        .lines()
        .find_map(|line| line.strip_prefix("gitdir: "))
        .ok_or_else(|| {
            WorkbenchError::Parse(format!(
                "unexpected .git file format in {}",
                worktree_path.display()
            ))
        })?
        .trim()
        .to_string();

    let gitdir_path = PathBuf::from(gitdir);
    let main_repo = gitdir_path
        .parent() // drops <worktree-name>
        .and_then(|p| p.parent()) // drops "worktrees"
        .and_then(|p| p.parent()) // drops ".git"
        .ok_or_else(|| {
            WorkbenchError::Parse(format!(
                "could not derive main repo from gitdir path: {}",
                gitdir_path.display()
            ))
        })?
        .to_path_buf();

    Ok(main_repo)
}

/// Find the most recent `AgentRun` with a matching branch name.
async fn find_run_by_branch(
    store: &rat_store::store::Store,
    branch: &str,
) -> Result<Option<AgentRun>> {
    let runs = store
        .recent_agent_runs(50)
        .await
        .map_err(|e| WorkbenchError::Parse(format!("store error: {e}")))?;
    Ok(runs.into_iter().find(|r| r.branch == branch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_to_slug_basic() {
        assert_eq!(title_to_slug("Fix auth bug"), "fix-auth-bug");
    }

    #[test]
    fn title_to_slug_special_chars() {
        assert_eq!(title_to_slug("  add   CI/CD  "), "add-ci-cd");
    }

    #[test]
    fn title_to_slug_empty() {
        assert_eq!(title_to_slug(""), "task");
    }

    #[test]
    fn shell_escape_path_plain() {
        let p = Path::new("/home/user/repo");
        assert_eq!(shell_escape_path(p), "'/home/user/repo'");
    }

    #[test]
    fn shell_escape_path_with_space() {
        let p = Path::new("/home/user/my repo");
        assert_eq!(shell_escape_path(p), "'/home/user/my repo'");
    }

    #[test]
    fn docker_run_command_mounts_worktree_and_quotes_args() {
        let cmd = docker_run_command(
            "codex:test",
            Path::new("/home/user/my repo"),
            "codex exec 'fix bug'",
        );
        assert_eq!(
            cmd,
            "docker run --rm -i -v '/home/user/my repo':/workspace -w /workspace 'codex:test' sh -lc 'codex exec '\\''fix bug'\\'''"
        );
    }

    #[test]
    fn wrapped_local_command_preserves_existing_cwd_behavior() {
        let adapter = crate::adapter::FakeAgent::new(Path::new("/tmp/rat-workbench-fixtures"));
        let cmd = wrapped_run_command(
            "fix bug",
            &adapter,
            Path::new("/home/user/repo"),
            &ExecutionBackend::Local,
        );
        assert_eq!(
            cmd,
            "cd '/home/user/repo' && bash /tmp/rat-workbench-fixtures/tests/fixtures/fakeagent.sh; echo RATO_EXIT=$?"
        );
    }

    #[test]
    fn wrapped_docker_command_uses_container_adapter_command() {
        let adapter = crate::adapter::FakeAgent::new(Path::new("/tmp/rat-workbench-fixtures"));
        let cmd = wrapped_run_command(
            "fix bug",
            &adapter,
            Path::new("/home/user/repo"),
            &ExecutionBackend::Docker {
                image: "agent:latest".to_string(),
            },
        );
        assert_eq!(
            cmd,
            "docker run --rm -i -v '/home/user/repo':/workspace -w /workspace 'agent:latest' sh -lc 'echo fakeagent fixture is host-only; use a docker image with the desired agent installed; exit 2'; echo RATO_EXIT=$?"
        );
    }
}
