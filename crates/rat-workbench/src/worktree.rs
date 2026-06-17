use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::error::{Result, WorkbenchError};
use rat_core::paths::data_dir;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A checked-out git worktree managed by rato.
#[derive(Debug, Clone)]
pub struct Worktree {
    /// Absolute path to the worktree directory.
    pub path: PathBuf,
    /// Branch name (`rato/<slug>`).
    pub branch: String,
    /// The base ref the branch was cut from.
    pub base: String,
}

/// Outcome of attempting to merge a worktree branch back into the live repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    /// The branch was successfully merged; contains the new HEAD sha.
    Merged { commit_sha: String },
    /// The branch cannot be fast-merged (conflicts present) — manual
    /// intervention is required. The live repo is untouched.
    NeedsManual,
}

// ---------------------------------------------------------------------------
// Repo-hash helper
// ---------------------------------------------------------------------------

/// First 12 hex characters of SHA-256 of the canonical repo path string.
pub fn repo_hash(repo_root: &Path) -> Result<String> {
    let canonical = repo_root
        .canonicalize()
        .map_err(|e| WorkbenchError::Canonicalize {
            path: repo_root.display().to_string(),
            source: e,
        })?;
    let path_str = canonical
        .to_str()
        .ok_or_else(|| WorkbenchError::Parse("repo path is not valid UTF-8".into()))?;
    let mut hasher = Sha256::new();
    hasher.update(path_str.as_bytes());
    let result = hasher.finalize();
    let hex: String = result.iter().map(|b| format!("{:02x}", b)).collect();
    Ok(hex[..12].to_string())
}

// ---------------------------------------------------------------------------
// Git helper
// ---------------------------------------------------------------------------

fn git_in(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    if out.status.success() {
        Ok(String::from_utf8(out.stdout)?)
    } else {
        Err(WorkbenchError::GitFailed {
            code: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

fn git_in_raw(dir: &Path, args: &[&str]) -> Result<(bool, Vec<u8>, Vec<u8>)> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    Ok((out.status.success(), out.stdout, out.stderr))
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create a new worktree at `~/.local/share/rato/worktrees/<repo-hash>/<task-id>/`.
///
/// Branch `rato/<slug>` is cut from `base`.
///
/// The `base` symbolic ref (e.g. `"HEAD"`, `"main"`) is resolved to its
/// commit SHA immediately so that `diffstat` / `commits_ahead` produce stable
/// ranges even after HEAD moves inside the worktree.
pub fn create(repo_root: &Path, task_id: &str, slug: &str, base: &str) -> Result<Worktree> {
    let hash = repo_hash(repo_root)?;
    let worktrees_dir = data_dir().join("worktrees").join(&hash).join(task_id);

    if worktrees_dir.exists() {
        return Err(WorkbenchError::WorktreeExists(
            worktrees_dir.display().to_string(),
        ));
    }

    // Resolve the base ref to a commit SHA so it stays stable.
    let base_sha = git_in(repo_root, &["rev-parse", base])?.trim().to_string();

    let branch = format!("rato/{}", slug);
    let path_str = worktrees_dir
        .to_str()
        .ok_or_else(|| WorkbenchError::Parse("worktree path is not valid UTF-8".into()))?;

    git_in(
        repo_root,
        &["worktree", "add", "-b", &branch, path_str, base],
    )?;

    Ok(Worktree {
        path: worktrees_dir,
        branch,
        base: base_sha,
    })
}

/// `git diff --stat <base>...HEAD` run inside the worktree.
pub fn diffstat(w: &Worktree) -> Result<String> {
    let range = format!("{}...HEAD", w.base);
    git_in(&w.path, &["diff", "--stat", &range])
}

/// Full `git diff <base>...HEAD` run inside the worktree.
pub fn full_diff(w: &Worktree) -> Result<String> {
    let range = format!("{}...HEAD", w.base);
    git_in(&w.path, &["diff", &range])
}

/// Number of commits ahead of `base` on the worktree's branch.
pub fn commits_ahead(w: &Worktree) -> Result<u32> {
    let range = format!("{}..HEAD", w.base);
    let out = git_in(&w.path, &["rev-list", "--count", &range])?;
    out.trim()
        .parse::<u32>()
        .map_err(|e| WorkbenchError::Parse(format!("could not parse commit count: {}", e)))
}

/// Remove the worktree directory but **keep** the branch.
///
/// `git worktree remove` must be run from the **main** repo, not from the
/// worktree itself (which would fail) and not from an unrelated directory
/// that is not a git repo.  We locate the main repo by reading the worktree's
/// `.git` file which contains a line like:
///   `gitdir: /path/to/main/.git/worktrees/<name>`
/// Stripping `/.git/worktrees/<name>` yields the main repo root.
pub fn remove(w: &Worktree) -> Result<()> {
    let main_repo = find_main_repo(&w.path)?;
    let path_str = w
        .path
        .to_str()
        .ok_or_else(|| WorkbenchError::Parse("worktree path is not valid UTF-8".into()))?;
    git_in(&main_repo, &["worktree", "remove", "--force", path_str])?;
    Ok(())
}

/// Parse `<worktree>/.git` (a file) to find the main repo root.
///
/// The file contains exactly `gitdir: <path>/.git/worktrees/<name>`.
/// We strip the trailing `/.git/worktrees/<name>` components to obtain the
/// main repo root.
fn find_main_repo(worktree_path: &Path) -> Result<PathBuf> {
    let git_file = worktree_path.join(".git");
    let content = std::fs::read_to_string(&git_file).map_err(WorkbenchError::Io)?;
    // Format: "gitdir: /absolute/path/.git/worktrees/<name>\n"
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

    // Strip "/.git/worktrees/<name>" — that's 3 path components from the end.
    // More precisely, the gitdir path looks like:
    //   /main/repo/.git/worktrees/<worktree-name>
    // so we go up two levels from that path and then one more to leave .git,
    // i.e. parent().parent().parent() of the gitdir.
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

// ---------------------------------------------------------------------------
// Merge helpers
// ---------------------------------------------------------------------------

/// Detect git version at runtime, returning `(major, minor)`.
fn git_version() -> (u32, u32) {
    let out = Command::new("git").arg("--version").output();
    let text = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => return (0, 0),
    };
    // "git version 2.53.0" or "git version 2.38.1"
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() < 3 {
        return (0, 0);
    }
    let nums: Vec<u32> = parts[2]
        .split('.')
        .take(2)
        .map(|s| s.parse().unwrap_or(0))
        .collect();
    (
        nums.first().copied().unwrap_or(0),
        nums.get(1).copied().unwrap_or(0),
    )
}

/// `true` if `git merge-tree --write-tree` is available (git ≥ 2.38).
fn has_merge_tree() -> bool {
    let (maj, min) = git_version();
    maj > 2 || (maj == 2 && min >= 38)
}

/// Check whether `branch` can be merged into the current HEAD of `repo_root`
/// without conflicts.
///
/// Primary path: `git merge-tree --write-tree HEAD <branch>` (git ≥ 2.38).
/// Fallback: dry-run `git merge --no-commit --no-ff` in a temp clone.
pub fn is_fast_mergeable(repo_root: &Path, branch: &str) -> Result<bool> {
    if has_merge_tree() {
        // git merge-tree --write-tree HEAD <branch>
        // Exit 0 and no conflict markers → clean merge.
        let (success, stdout, _stderr) =
            git_in_raw(repo_root, &["merge-tree", "--write-tree", "HEAD", branch])?;

        if !success {
            return Ok(false);
        }

        // The output contains the new tree SHA. Conflict markers may appear on
        // additional lines. An unambiguous sign of a conflict is that merge-tree
        // exits non-zero OR writes lines containing "<<<<<<<"
        let output_str = String::from_utf8_lossy(&stdout);
        Ok(!output_str.contains("<<<<<<<"))
    } else {
        // Fallback: temp-clone dry-run
        is_fast_mergeable_via_clone(repo_root, branch)
    }
}

fn is_fast_mergeable_via_clone(repo_root: &Path, branch: &str) -> Result<bool> {
    use std::fs;

    let tmp = tempfile_dir()?;
    let clone_dir = tmp.join("rato-merge-check");

    // git clone --local <repo_root> <clone_dir>
    let (ok, _, _) = git_in_raw(
        repo_root,
        &[
            "clone",
            "--local",
            repo_root
                .to_str()
                .ok_or_else(|| WorkbenchError::Parse("bad path".into()))?,
            clone_dir
                .to_str()
                .ok_or_else(|| WorkbenchError::Parse("bad clone path".into()))?,
        ],
    )?;
    if !ok {
        let _ = fs::remove_dir_all(&clone_dir);
        return Ok(false);
    }

    // Fetch the branch from origin into the clone
    git_in(&clone_dir, &["fetch", "origin", branch])?;
    let _ = git_in(&clone_dir, &["checkout", "-b", branch, "FETCH_HEAD"]);

    // Try the merge dry-run
    let result = git_in_raw(
        &clone_dir,
        &["merge", "--no-commit", "--no-ff", "FETCH_HEAD"],
    )?;
    // Abort the merge to leave clone clean (not strictly necessary but tidy)
    let _ = git_in(&clone_dir, &["merge", "--abort"]);
    let _ = fs::remove_dir_all(&clone_dir);

    Ok(result.0)
}

/// A quick helper: create a unique temp directory without bringing in `tempfile`
/// as a non-dev dependency.
fn tempfile_dir() -> Result<PathBuf> {
    let base = std::env::temp_dir().join(format!("rato-wt-{}", std::process::id()));
    std::fs::create_dir_all(&base)?;
    Ok(base)
}

/// Merge `branch` into the current HEAD of `repo_root` using `--no-ff --no-edit`.
///
/// Returns `MergeOutcome::NeedsManual` if `is_fast_mergeable` says it would
/// conflict, without touching the repo.
pub fn merge_into_live(repo_root: &Path, branch: &str) -> Result<MergeOutcome> {
    if !is_fast_mergeable(repo_root, branch)? {
        return Ok(MergeOutcome::NeedsManual);
    }

    git_in(repo_root, &["merge", "--no-ff", "--no-edit", branch])?;

    // Read the new HEAD sha
    let sha = git_in(repo_root, &["rev-parse", "HEAD"])?
        .trim()
        .to_string();

    Ok(MergeOutcome::Merged { commit_sha: sha })
}

// ---------------------------------------------------------------------------
// Escape guard
// ---------------------------------------------------------------------------

/// Assert that `cwd` is inside `worktree_root`.
///
/// Both paths are canonicalized before comparison, so symlinks cannot bypass
/// the check. Returns `Err(WorkbenchError::EscapeGuard)` if `cwd` is not a
/// descendant of `worktree_root`.
pub fn escape_guard(worktree_root: &Path, cwd: &Path) -> Result<()> {
    let root = worktree_root
        .canonicalize()
        .map_err(|e| WorkbenchError::Canonicalize {
            path: worktree_root.display().to_string(),
            source: e,
        })?;
    let current = cwd
        .canonicalize()
        .map_err(|e| WorkbenchError::Canonicalize {
            path: cwd.display().to_string(),
            source: e,
        })?;

    if current.starts_with(&root) {
        Ok(())
    } else {
        Err(WorkbenchError::EscapeGuard {
            cwd: current.display().to_string(),
            root: root.display().to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Env scrub
// ---------------------------------------------------------------------------

/// Remove sensitive / misleading environment variables from a `Command` so
/// that an agent subprocess cannot inherit them.
///
/// Variables removed:
/// - `SSH_AUTH_SOCK` — prevents agents from piggybacking on the user's SSH agent
/// - `GIT_DIR` — prevents git from operating on an unexpected repository
/// - `GIT_WORK_TREE` — same concern
pub fn scrub_agent_env(cmd: &mut Command) {
    cmd.env_remove("SSH_AUTH_SOCK")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
}
