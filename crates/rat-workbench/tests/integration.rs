/// Integration tests for rat-workbench.
///
/// Each test is guarded by a runtime check for `tmux` / `git` on PATH.
/// If either binary is absent the test is skipped so CI without these tools
/// stays green.
///
/// Tmux servers use a UNIQUE `-L rato-test-<pid>` socket so that concurrent
/// test runs never interfere with each other (or with a real rato server).
/// Every test kills its server in teardown.
use std::path::Path;
use std::process::Command;

use rat_workbench::tmux::Tmux;
use rat_workbench::worktree::{self, escape_guard, MergeOutcome};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn has_binary(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create a throwaway git repo with one commit and return the tempdir handle.
fn make_repo(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@test.local"]);
    git(&repo, &["config", "user.name", "Test"]);
    // Seed commit
    std::fs::write(repo.join("README.md"), "# rato test repo\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "init"]);
    repo
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git failed");
    assert!(status.success(), "git {:?} failed in {}", args, dir.display());
}

fn git_output(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git output failed");
    String::from_utf8(out.stdout).unwrap()
}

/// Unique tmux socket name for this process.
fn test_socket() -> String {
    format!("rato-test-{}", std::process::id())
}

// ---------------------------------------------------------------------------
// Test: full tmux lifecycle
// ---------------------------------------------------------------------------

#[test]
fn test_tmux_lifecycle() {
    if !has_binary("tmux") || !has_binary("git") {
        eprintln!("skipping: tmux or git not found");
        return;
    }

    let socket = test_socket();
    let tmux = Tmux::new(&socket);

    // Teardown guard via a closure we call explicitly at the end (and in every
    // panic path via the Drop implementation below).
    struct Guard(Tmux);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.kill_server();
        }
    }
    let _guard = Guard(tmux.clone());

    // Start server + session
    tmux.ensure_server().expect("ensure_server");
    tmux.ensure_session("test-sess").expect("ensure_session");

    // Second call to ensure_session must be idempotent
    tmux.ensure_session("test-sess").expect("ensure_session (2nd)");

    // Create a window
    let tmp = tempfile::TempDir::new().unwrap();
    let target = tmux
        .new_window("test-sess", "w1", tmp.path())
        .expect("new_window");
    assert_eq!(target, "test-sess:w1");

    // Window should be alive
    assert!(tmux.window_alive(&target), "window should be alive");

    // Run a command and capture its output
    tmux.run_in_window(&target, "echo HELLO_RATO").expect("run_in_window");

    // Give the shell a moment to flush output
    std::thread::sleep(std::time::Duration::from_millis(400));

    let output = tmux.capture_tail(&target, 20).expect("capture_tail");
    assert!(
        output.contains("HELLO_RATO"),
        "expected HELLO_RATO in output, got: {:?}",
        output
    );

    // Kill the window
    tmux.kill_window(&target).expect("kill_window");
    assert!(!tmux.window_alive(&target), "window should be dead after kill");
}

// ---------------------------------------------------------------------------
// Test: worktree create / diffstat / remove
// ---------------------------------------------------------------------------

#[test]
fn test_worktree_lifecycle() {
    if !has_binary("git") {
        eprintln!("skipping: git not found");
        return;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let repo = make_repo(&tmp);

    // Create worktree
    let wt = worktree::create(&repo, "t001", "my-task", "HEAD")
        .expect("create worktree");

    assert!(wt.path.exists(), "worktree path should exist");
    assert_eq!(wt.branch, "rato/my-task");

    // Make a commit in the worktree
    std::fs::write(wt.path.join("agent_note.txt"), "hello from agent\n").unwrap();
    git(&wt.path, &["config", "user.email", "test@test.local"]);
    git(&wt.path, &["config", "user.name", "Test"]);
    git(&wt.path, &["add", "agent_note.txt"]);
    git(&wt.path, &["commit", "-m", "agent: add note"]);

    // diffstat should mention agent_note.txt
    let stat = worktree::diffstat(&wt).expect("diffstat");
    assert!(
        stat.contains("agent_note.txt"),
        "diffstat should mention agent_note.txt, got: {:?}",
        stat
    );

    // full_diff should contain the file content
    let diff = worktree::full_diff(&wt).expect("full_diff");
    assert!(
        diff.contains("hello from agent"),
        "full diff should contain file content"
    );

    // commits_ahead should be 1
    let ahead = worktree::commits_ahead(&wt).expect("commits_ahead");
    assert_eq!(ahead, 1, "should be 1 commit ahead");

    // Remove the worktree (keep branch)
    worktree::remove(&wt).expect("remove worktree");
    assert!(
        !wt.path.exists(),
        "worktree path should not exist after remove"
    );

    // Branch should still exist in the main repo
    let branches = git_output(&repo, &["branch"]);
    assert!(
        branches.contains("rato/my-task"),
        "branch should still exist after remove, branches: {:?}",
        branches
    );
}

// ---------------------------------------------------------------------------
// Test: escape_guard refuses /tmp
// ---------------------------------------------------------------------------

#[test]
fn test_escape_guard_rejects_outside() {
    if !has_binary("git") {
        eprintln!("skipping: git not found");
        return;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let repo = make_repo(&tmp);

    // /tmp is definitely outside the worktree
    let result = escape_guard(&repo, Path::new("/tmp"));
    assert!(
        result.is_err(),
        "escape_guard should reject /tmp as cwd outside worktree root"
    );
}

#[test]
fn test_escape_guard_accepts_inside() {
    if !has_binary("git") {
        eprintln!("skipping: git not found");
        return;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let repo = make_repo(&tmp);

    // repo itself is inside itself
    let result = escape_guard(&repo, &repo);
    assert!(result.is_ok(), "escape_guard should accept cwd == root");

    // a subdir also works
    let sub = repo.join("subdir");
    std::fs::create_dir_all(&sub).unwrap();
    let result = escape_guard(&repo, &sub);
    assert!(result.is_ok(), "escape_guard should accept cwd inside root");
}

// ---------------------------------------------------------------------------
// Test: full create → tmux run_in_window → capture_tail → remove cycle
// ---------------------------------------------------------------------------

#[test]
fn test_full_cycle_with_tmux() {
    if !has_binary("tmux") || !has_binary("git") {
        eprintln!("skipping: tmux or git not found");
        return;
    }

    let socket = format!("rato-test-full-{}", std::process::id());
    let tmux = Tmux::new(&socket);

    struct Guard(Tmux);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.kill_server();
        }
    }
    let _guard = Guard(tmux.clone());

    let tmp = tempfile::TempDir::new().unwrap();
    let repo = make_repo(&tmp);

    // Create worktree
    let wt = worktree::create(&repo, "t002", "full-cycle", "HEAD")
        .expect("create worktree for full cycle");

    // Boot tmux
    tmux.ensure_server().expect("ensure_server");
    tmux.ensure_session("full-cycle-sess").expect("ensure_session");

    // Open a window inside the worktree
    let target = tmux
        .new_window("full-cycle-sess", "wfull", &wt.path)
        .expect("new_window");

    // Run a command that produces distinct output
    tmux.run_in_window(&target, "echo CYCLE_DONE").expect("run_in_window");
    std::thread::sleep(std::time::Duration::from_millis(400));

    let captured = tmux.capture_tail(&target, 20).expect("capture_tail");
    assert!(
        captured.contains("CYCLE_DONE"),
        "capture should contain CYCLE_DONE, got: {:?}",
        captured
    );

    // Clean up tmux window
    tmux.kill_window(&target).expect("kill_window");

    // Remove worktree
    worktree::remove(&wt).expect("remove worktree after full cycle");
}

// ---------------------------------------------------------------------------
// Test: merge_into_live + MergeOutcome
// ---------------------------------------------------------------------------

#[test]
fn test_merge_into_live() {
    if !has_binary("git") {
        eprintln!("skipping: git not found");
        return;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let repo = make_repo(&tmp);

    // Capture HEAD before merge
    let head_before = git_output(&repo, &["rev-parse", "HEAD"]).trim().to_string();

    // Create a worktree and make a commit
    let wt = worktree::create(&repo, "t003", "merge-test", "HEAD")
        .expect("create worktree for merge test");

    std::fs::write(wt.path.join("merged_file.txt"), "merged content\n").unwrap();
    git(&wt.path, &["config", "user.email", "test@test.local"]);
    git(&wt.path, &["config", "user.name", "Test"]);
    git(&wt.path, &["add", "merged_file.txt"]);
    git(&wt.path, &["commit", "-m", "agent: add merged file"]);

    // The branch name without "refs/heads/" prefix
    let branch = wt.branch.clone();

    // Remove the worktree (but keep branch) before merging
    worktree::remove(&wt).expect("remove before merge");

    // Configure git in main repo to allow merges
    git(&repo, &["config", "user.email", "test@test.local"]);
    git(&repo, &["config", "user.name", "Test"]);

    let outcome = worktree::merge_into_live(&repo, &branch).expect("merge_into_live");

    match outcome {
        MergeOutcome::Merged { commit_sha } => {
            assert_ne!(commit_sha, head_before, "HEAD should have moved after merge");
            // Verify merged file is present in main repo
            assert!(
                repo.join("merged_file.txt").exists(),
                "merged_file.txt should exist in main repo after merge"
            );
        }
        MergeOutcome::NeedsManual => {
            panic!("expected clean merge, got NeedsManual");
        }
    }
}
