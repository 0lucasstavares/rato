use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadState {
    /// None when HEAD is detached.
    pub branch: Option<String>,
    pub commit: String,
}

/// Read a repo's HEAD purely from files — no subprocess. Handles plain `.git`
/// dirs, worktree `.git` files (`gitdir:` indirection + `commondir`), loose
/// refs and packed-refs.
pub fn read_head(root: &Path) -> Option<HeadState> {
    let gitdir = resolve_gitdir(root)?;
    let head = std::fs::read_to_string(gitdir.join("HEAD")).ok()?;
    let head = head.trim();

    if let Some(refname) = head.strip_prefix("ref: ") {
        let branch = refname.strip_prefix("refs/heads/").map(|s| s.to_string());
        // refs live in the common dir for worktrees
        let refs_base = match std::fs::read_to_string(gitdir.join("commondir")) {
            Ok(common) => {
                let p = PathBuf::from(common.trim());
                if p.is_absolute() {
                    p
                } else {
                    gitdir.join(p)
                }
            }
            Err(_) => gitdir.clone(),
        };
        let commit = read_ref(&refs_base, refname)?;
        Some(HeadState { branch, commit })
    } else if head.len() >= 40 {
        Some(HeadState { branch: None, commit: head.to_string() })
    } else {
        None
    }
}

fn resolve_gitdir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    // worktree: `.git` is a file "gitdir: <path>"
    let contents = std::fs::read_to_string(&dot_git).ok()?;
    let target = contents.trim().strip_prefix("gitdir: ")?;
    let p = PathBuf::from(target);
    Some(if p.is_absolute() { p } else { root.join(p) })
}

fn read_ref(refs_base: &Path, refname: &str) -> Option<String> {
    if let Ok(content) = std::fs::read_to_string(refs_base.join(refname)) {
        return Some(content.trim().to_string());
    }
    // fall back to packed-refs
    let packed = std::fs::read_to_string(refs_base.join("packed-refs")).ok()?;
    for line in packed.lines() {
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        if let Some((hash, name)) = line.split_once(' ') {
            if name.trim() == refname {
                return Some(hash.trim().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn reads_branch_and_commit_from_real_repo() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q", "-b", "main"]);
        git(tmp.path(), &["commit", "--allow-empty", "-q", "-m", "one"]);

        let head = read_head(tmp.path()).unwrap();
        assert_eq!(head.branch.as_deref(), Some("main"));
        assert_eq!(head.commit.len(), 40);

        // new commit changes the state
        git(tmp.path(), &["commit", "--allow-empty", "-q", "-m", "two"]);
        let head2 = read_head(tmp.path()).unwrap();
        assert_ne!(head.commit, head2.commit);
        assert_eq!(head2.branch.as_deref(), Some("main"));
    }

    #[test]
    fn detached_head_has_no_branch() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q", "-b", "main"]);
        git(tmp.path(), &["commit", "--allow-empty", "-q", "-m", "one"]);
        git(tmp.path(), &["checkout", "-q", "--detach"]);

        let head = read_head(tmp.path()).unwrap();
        assert_eq!(head.branch, None);
        assert_eq!(head.commit.len(), 40);
    }

    #[test]
    fn packed_refs_are_resolved() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q", "-b", "main"]);
        git(tmp.path(), &["commit", "--allow-empty", "-q", "-m", "one"]);
        git(tmp.path(), &["pack-refs", "--all"]);

        let head = read_head(tmp.path()).unwrap();
        assert_eq!(head.branch.as_deref(), Some("main"));
        assert_eq!(head.commit.len(), 40);
    }

    #[test]
    fn non_repo_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_head(tmp.path()), None);
    }
}
