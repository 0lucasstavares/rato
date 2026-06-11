//! Integration tests for TaskRunner, AgentAdapter, and the fakeagent fixture.
//!
//! Each test is guarded by a runtime check for `tmux` and `git` on PATH.
//! If either binary is absent the test is skipped so CI without these tools
//! stays green.
//!
//! Tests use isolated tmux sockets (`rato-runner-test-<pid>-<n>`) and
//! tempdir git repos to prevent cross-test contamination.
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rat_core::clock::{Clock, FakeClock};
use rat_store::store::Store;
use rat_workbench::adapter::FakeAgent;
use rat_workbench::runner::TaskRunner;
use rat_workbench::tmux::Tmux;
use rat_workbench::worktree::MergeOutcome;

// ---------------------------------------------------------------------------
// Test counter to produce unique tmux socket names per test
// ---------------------------------------------------------------------------

static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_socket() -> String {
    let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("rato-runner-{}-{}", std::process::id(), n)
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

fn has_binary(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn require_binaries() -> bool {
    if !has_binary("tmux") || !has_binary("git") {
        eprintln!("skipping: tmux or git not found on PATH");
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Repo / store helpers
// ---------------------------------------------------------------------------

fn make_repo(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@test.local"]);
    git(&repo, &["config", "user.name", "Test"]);
    std::fs::write(repo.join("README.md"), "# rato runner test\n").unwrap();
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

fn open_test_store(tmp: &tempfile::TempDir, clock: Arc<FakeClock>) -> Store {
    Store::open(&tmp.path().join("test.db"), clock).expect("open store")
}

// ---------------------------------------------------------------------------
// Poll helper: retry poll() until a terminal status is reached (or timeout).
// ---------------------------------------------------------------------------

async fn poll_until_done(runner: &TaskRunner, run_id: &str) {
    for _ in 0..60 {
        let run = runner.poll(run_id).await.expect("poll failed");
        if let Some(ref r) = run {
            if r.status == "done" || r.status == "failed" {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    panic!("run {run_id} did not reach terminal status within 18 seconds");
}

// ---------------------------------------------------------------------------
// Test: fakeagent e2e — start → poll until done → merge_back → approve →
//   execute_merge → live repo HEAD moved, AGENT_NOTE.md present
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fakeagent_e2e_approve_and_merge() {
    if !require_binaries() {
        return;
    }

    let socket = unique_socket();
    let tmux = Tmux::new(&socket);

    struct Guard(Tmux);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.kill_server();
        }
    }
    let _guard = Guard(tmux.clone());

    let tmp_repo = tempfile::TempDir::new().unwrap();
    let tmp_store = tempfile::TempDir::new().unwrap();
    let repo = make_repo(&tmp_repo);

    let clock = FakeClock::at(1_000_000);
    let store = open_test_store(&tmp_store, clock.clone());

    // Set up project in store.
    let project = store
        .upsert_project(repo.display().to_string(), "test-project".into())
        .await
        .expect("upsert project");

    // Configure git in the repo so merge commits don't fail.
    git(&repo, &["config", "user.email", "test@test.local"]);
    git(&repo, &["config", "user.name", "Test"]);

    let runner = TaskRunner::new(store.clone(), tmux, clock.clone());
    let adapter = FakeAgent::from_manifest();

    // Start the run.
    let run = runner
        .start(&repo, &project.id, "fakeagent task", &adapter, "HEAD")
        .await
        .expect("start failed");

    assert_eq!(run.status, "running");
    assert!(run.tmux_target.is_some());

    // Poll until done.
    poll_until_done(&runner, &run.id).await;

    let finished = store
        .get_agent_run(run.id.clone())
        .await
        .expect("get run")
        .expect("run should exist");
    assert_eq!(finished.status, "done", "run should be done, got: {}", finished.status);

    // Capture HEAD before merge.
    let head_before = git_output(&repo, &["rev-parse", "HEAD"]).trim().to_string();

    // Create merge_back approval.
    let approval = runner.merge_back(&run.id).await.expect("merge_back failed");
    assert_eq!(approval.status, "pending");
    assert_eq!(approval.risk, 2, "MergeBack must be R2");

    // Approve the approval via store (simulates operator decision).
    let now = clock.now_ms();
    let approved = store
        .decide_approval(
            approval.id.clone(),
            "approved".into(),
            now,
            "cli".into(),
            None,
        )
        .await
        .expect("decide_approval failed");
    assert_eq!(approved.status, "approved");

    // Execute the merge.
    let outcome = runner
        .execute_merge(&approved)
        .await
        .expect("execute_merge failed");

    match outcome {
        MergeOutcome::Merged { ref commit_sha } => {
            // HEAD must have moved.
            let head_after = git_output(&repo, &["rev-parse", "HEAD"]).trim().to_string();
            assert_ne!(
                head_after, head_before,
                "HEAD should move after merge"
            );
            assert!(!commit_sha.is_empty());

            // AGENT_NOTE.md must be present in the live repo.
            assert!(
                repo.join("AGENT_NOTE.md").exists(),
                "AGENT_NOTE.md should be present in live repo after merge"
            );
        }
        MergeOutcome::NeedsManual => {
            panic!("expected clean merge, got NeedsManual");
        }
    }

    // The approval should have an execution record.
    let final_approval = store
        .get_approval(approved.id.clone())
        .await
        .expect("get approval")
        .expect("approval should exist");
    assert!(
        final_approval.execution.is_some(),
        "approval should have execution metadata"
    );

    // The run should be marked merged.
    let final_run = store
        .get_agent_run(run.id.clone())
        .await
        .expect("get run")
        .expect("run should exist");
    assert_eq!(final_run.status, "merged", "run status should be 'merged'");
}

// ---------------------------------------------------------------------------
// Test: deny path — live repo HEAD and status are byte-identical before/after.
// This is the M4 hard invariant.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fakeagent_deny_leaves_live_repo_unchanged() {
    if !require_binaries() {
        return;
    }

    let socket = unique_socket();
    let tmux = Tmux::new(&socket);

    struct Guard(Tmux);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.kill_server();
        }
    }
    let _guard = Guard(tmux.clone());

    let tmp_repo = tempfile::TempDir::new().unwrap();
    let tmp_store = tempfile::TempDir::new().unwrap();
    let repo = make_repo(&tmp_repo);

    let clock = FakeClock::at(2_000_000);
    let store = open_test_store(&tmp_store, clock.clone());

    let project = store
        .upsert_project(repo.display().to_string(), "deny-test-project".into())
        .await
        .expect("upsert project");

    git(&repo, &["config", "user.email", "test@test.local"]);
    git(&repo, &["config", "user.name", "Test"]);

    let runner = TaskRunner::new(store.clone(), tmux, clock.clone());
    let adapter = FakeAgent::from_manifest();

    // Snapshot BEFORE the run.
    let head_before = git_output(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    let status_before = git_output(&repo, &["status", "--porcelain"]).trim().to_string();

    // Start a run.
    let run = runner
        .start(&repo, &project.id, "deny test task", &adapter, "HEAD")
        .await
        .expect("start failed");

    // Poll until done.
    poll_until_done(&runner, &run.id).await;

    // Create merge_back approval.
    let approval = runner.merge_back(&run.id).await.expect("merge_back failed");

    // Deny the approval.
    let denied = runner
        .deny(&approval.id, Some("test denial"))
        .await
        .expect("deny failed");
    assert_eq!(denied.status, "denied");

    // Snapshot AFTER deny.
    let head_after = git_output(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    let status_after = git_output(&repo, &["status", "--porcelain"]).trim().to_string();

    // INVARIANT: live repo must be byte-identical.
    assert_eq!(
        head_before, head_after,
        "HEAD must not move after denial (deny invariant)"
    );
    assert_eq!(
        status_before, status_after,
        "git status --porcelain must be identical after denial (deny invariant)"
    );

    // The branch must still exist.
    let branches = git_output(&repo, &["branch"]);
    assert!(
        branches.contains(&run.branch.replace("refs/heads/", "")),
        "branch {} should still exist after denial, branches: {:?}",
        run.branch,
        branches
    );
}

// ---------------------------------------------------------------------------
// Test: execute_merge on a conflicting branch → NeedsManual.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_merge_returns_needs_manual_on_conflict() {
    if !has_binary("git") {
        eprintln!("skipping: git not found");
        return;
    }

    let tmp_repo = tempfile::TempDir::new().unwrap();
    let tmp_store = tempfile::TempDir::new().unwrap();
    let repo = make_repo(&tmp_repo);

    let clock = FakeClock::at(3_000_000);
    let store = open_test_store(&tmp_store, clock.clone());

    git(&repo, &["config", "user.email", "test@test.local"]);
    git(&repo, &["config", "user.name", "Test"]);

    // Create a worktree branch and make a conflicting commit.
    let wt = rat_workbench::worktree::create(&repo, "conflict-task", "conflict-branch", "HEAD")
        .expect("create worktree");

    // Commit in worktree — touches README.md.
    git(&wt.path, &["config", "user.email", "test@test.local"]);
    git(&wt.path, &["config", "user.name", "Test"]);
    std::fs::write(wt.path.join("README.md"), "worktree version\n").unwrap();
    git(&wt.path, &["add", "README.md"]);
    git(&wt.path, &["commit", "-m", "worktree: edit readme"]);

    // Also commit in the main repo — touches the same file to create a conflict.
    std::fs::write(repo.join("README.md"), "main version\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "main: edit readme"]);

    // Now the branch is not fast-mergeable.
    // Build a synthetic approval that points at this branch/repo.
    let expires_at = clock.now_ms() + 60 * 60 * 1000;
    let raw_approval = store
        .insert_approval(rat_store::rows::NewApproval {
            kind: "merge_back".into(),
            risk: 2,
            title: "Merge conflict test".into(),
            reason: "testing NeedsManual path".into(),
            cwd: None,
            target: Some(repo.display().to_string()),
            agent_identity: "fakeagent".into(),
            payload: serde_json::json!({
                "branch": wt.branch,
                "target": repo.display().to_string(),
                "diffstat": "",
                "diff": { "inline": "" }
            }),
            expected_impact: serde_json::json!({}),
            expires_at,
        })
        .await
        .expect("insert approval");

    // Approve it.
    let approved = store
        .decide_approval(
            raw_approval.id.clone(),
            "approved".into(),
            clock.now_ms(),
            "cli".into(),
            None,
        )
        .await
        .expect("decide_approval");

    // Build a minimal TaskRunner (tmux isn't needed for execute_merge).
    let socket = unique_socket();
    let tmux = Tmux::new(&socket);
    let runner = TaskRunner::new(store, tmux, clock);

    let outcome = runner.execute_merge(&approved).await.expect("execute_merge");
    assert_eq!(
        outcome,
        MergeOutcome::NeedsManual,
        "conflicting branch should yield NeedsManual"
    );
}

// ---------------------------------------------------------------------------
// Test: oversized diff (>32 KB) → blob path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_merge_back_large_diff_uses_blob() {
    if !require_binaries() {
        return;
    }

    let socket = unique_socket();
    let tmux = Tmux::new(&socket);

    struct Guard(Tmux);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.kill_server();
        }
    }
    let _guard = Guard(tmux.clone());

    let tmp_repo = tempfile::TempDir::new().unwrap();
    let tmp_store = tempfile::TempDir::new().unwrap();
    let repo = make_repo(&tmp_repo);

    let clock = FakeClock::at(4_000_000);
    let store = open_test_store(&tmp_store, clock.clone());

    let project = store
        .upsert_project(repo.display().to_string(), "blob-test-project".into())
        .await
        .expect("upsert project");

    git(&repo, &["config", "user.email", "test@test.local"]);
    git(&repo, &["config", "user.name", "Test"]);

    // Create a large-diff script: write a file bigger than 32KB.
    let big_content = "x".repeat(33 * 1024); // 33KB
    let script = format!("write:bigfile.txt:{big_content}\ncommit:big change\nexit:0");

    let runner = TaskRunner::new(store.clone(), tmux, clock.clone());

    // Manually create worktree and make a big commit (bypassing the FAKEAGENT_SCRIPT
    // env since tmux env injection is not straightforward here).
    // Instead, we create the worktree manually and write a big file.
    let task_id = format!("{:x}", clock.now_ms());
    let slug = "blob-test";
    let wt = rat_workbench::worktree::create(&repo, &task_id, slug, "HEAD")
        .expect("create worktree");

    // Write big file directly.
    std::fs::write(wt.path.join("bigfile.txt"), &big_content).unwrap();
    git(&wt.path, &["config", "user.email", "test@test.local"]);
    git(&wt.path, &["config", "user.name", "Test"]);
    git(&wt.path, &["add", "bigfile.txt"]);
    git(&wt.path, &["commit", "-m", "big change"]);

    // Insert a "done" AgentRun in the store manually so merge_back can find it.
    let run = store
        .insert_agent_run(rat_store::rows::NewAgentRun {
            adapter: "fakeagent".into(),
            task_title: "blob test task".into(),
            project_id: project.id.clone(),
            worktree_path: wt.path.display().to_string(),
            branch: wt.branch.clone(),
            tmux_target: None,
            mode: "headless".into(),
            tokens: serde_json::json!({}),
            cost_usd: 0.0,
            started: clock.now_ms(),
        })
        .await
        .expect("insert run");

    store
        .update_agent_run_status(run.id.clone(), "done".into(), Some(clock.now_ms()), None, None)
        .await
        .expect("update status");

    // Call merge_back — should detect diff > 32KB and store a blob.
    let approval = runner.merge_back(&run.id).await.expect("merge_back failed");

    // The diff payload should have a blob_id, not inline text.
    let diff_val = approval
        .payload
        .get("diff")
        .expect("approval payload should have 'diff'");
    let blob_id = diff_val
        .get("blob_id")
        .and_then(|v| v.as_str())
        .expect("diff should have 'blob_id' for large diffs");

    assert!(!blob_id.is_empty(), "blob_id should not be empty");

    // The blob must exist in the store.
    let blob = store
        .get_blob(blob_id.to_string())
        .await
        .expect("get blob")
        .expect("blob should exist in store");

    assert!(
        blob.bytes.len() > 32 * 1024,
        "blob should contain the large diff (>32KB)"
    );

    // Suppress unused variable warning for script (it was part of the original design intent).
    let _ = script;
}
