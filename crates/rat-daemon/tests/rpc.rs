use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use rat_core::clock::{Clock, FakeClock, SystemClock};
use rat_daemon::ingest::Ingest;
use rat_daemon::mode::ModeManager;
use rat_daemon::pins::{PinKeyStore, PinService};
use rat_daemon::server::{LlmStatusState, serve, ServerCtx};
use rat_daemon::sessionizer::{Sessionizer, DEFAULT_GAP_MS};
use rat_proto::{errcodes, Response, PROTO_VERSION};
use rat_ring::{Media, RingKey, RingWriter};
use rat_store::store::Store;
use rat_workbench::runner::TaskRunner;
use rat_workbench::tmux::Tmux;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

static TEST_SOCKET_CTR: AtomicU32 = AtomicU32::new(0);

fn unique_tmux_socket() -> String {
    let n = TEST_SOCKET_CTR.fetch_add(1, Ordering::SeqCst);
    format!("rato-rpctest-{}-{}", std::process::id(), n)
}

async fn start_with_store() -> (tempfile::TempDir, PathBuf, Store) {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("ratd.sock");
    let db = tmp.path().join("rato.db");
    let clock: Arc<dyn rat_core::clock::Clock> = Arc::new(SystemClock);
    let store = Store::open(&db, clock.clone()).unwrap();
    let ingest = Arc::new(Ingest::new(store.clone(), clock.clone(), Sessionizer::new(DEFAULT_GAP_MS)));
    let mode = Arc::new(ModeManager::new(0));
    let task_runner = TaskRunner::new(store.clone(), Tmux::new(format!("rato-test-{}", std::process::id())), clock.clone());
    let ctx = Arc::new(ServerCtx {
        store: store.clone(),
        ingest,
        mode,
        started: Instant::now(),
        db_path: db,
        clock,
        embedder: None,
        llm_status: LlmStatusState::disabled(),
        task_runner,
        pins: None,
    });
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(serve(listener, ctx));
    (tmp, socket, store)
}

async fn start() -> (tempfile::TempDir, PathBuf) {
    let (tmp, socket, _store) = start_with_store().await;
    (tmp, socket)
}

struct TestClient {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    w: tokio::net::unix::OwnedWriteHalf,
}

struct StaticPinKeyStore([u8; 32]);

impl PinKeyStore for StaticPinKeyStore {
    fn load_or_create(&self) -> anyhow::Result<[u8; 32]> {
        Ok(self.0)
    }
}

impl TestClient {
    async fn connect(socket: &Path) -> Self {
        let s = UnixStream::connect(socket).await.unwrap();
        let (r, w) = s.into_split();
        Self { lines: BufReader::new(r).lines(), w }
    }

    async fn send(&mut self, req: serde_json::Value) -> Response {
        let mut buf = serde_json::to_vec(&req).unwrap();
        buf.push(b'\n');
        self.w.write_all(&buf).await.unwrap();
        let line = self.lines.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

#[tokio::test]
async fn methods_before_hello_are_rejected() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    let resp = c.send(json!({"id": 1, "method": "status"})).await;
    assert_eq!(resp.error.unwrap().code, errcodes::HELLO_REQUIRED);
}

#[tokio::test]
async fn hello_with_wrong_proto_version_is_rejected() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    let resp = c
        .send(json!({"id": 1, "method": "hello", "params": {"proto_version": 999}}))
        .await;
    assert_eq!(resp.error.unwrap().code, errcodes::PROTO_MISMATCH);
    // and it did NOT unlock the connection
    let resp = c.send(json!({"id": 2, "method": "status"})).await;
    assert_eq!(resp.error.unwrap().code, errcodes::HELLO_REQUIRED);
}

#[tokio::test]
async fn full_round_trip_hello_status_append_recent() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;

    let hello = c
        .send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    assert!(hello.error.is_none());

    let status = c.send(json!({"id": 2, "method": "status"})).await;
    let status = status.result.unwrap();
    assert_eq!(status["event_count"], 0);

    let appended = c
        .send(json!({"id": 3, "method": "events.append",
                     "params": {"kind": "test_event", "source": "test", "payload": {"n": 1}}}))
        .await;
    let appended = appended.result.unwrap();
    assert_eq!(appended["kind"], "test_event");

    let recent =
        c.send(json!({"id": 4, "method": "events.recent", "params": {"limit": 10}})).await;
    let recent = recent.result.unwrap();
    assert_eq!(recent.as_array().unwrap().len(), 1);
    assert_eq!(recent[0]["payload"]["n"], 1);
}

#[tokio::test]
async fn shell_cmd_flows_into_projects_sessions_observations_via_rpc() {
    let (tmp, socket) = start().await;
    // a fake repo for project attribution
    let repo = tmp.path().join("acme");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;

    let appended = c
        .send(json!({"id": 2, "method": "events.append", "params": {
            "kind": "shell_cmd", "source": "shell",
            "payload": {"cmd": "cargo build", "cwd": repo.to_string_lossy(), "exit": 0, "duration_ms": 900}
        }}))
        .await;
    assert!(appended.result.unwrap()["project_id"].is_string());

    let projects = c.send(json!({"id": 3, "method": "projects.list"})).await.result.unwrap();
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["name"], "acme");

    let sessions = c.send(json!({"id": 4, "method": "sessions.recent"})).await.result.unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["commands"], 1);
    assert!(sessions[0]["ended"].is_null());

    let obs = c
        .send(json!({"id": 5, "method": "observations.recent", "params": {"kind": "shell_cmd"}}))
        .await
        .result
        .unwrap();
    assert_eq!(obs.as_array().unwrap().len(), 1);
    assert_eq!(obs[0]["content"], "cargo build");

    let mode = c.send(json!({"id": 6, "method": "mode.get"})).await.result.unwrap();
    assert_eq!(mode["mode"], "active");
}

#[tokio::test]
async fn rat_emit_loop_guard_returns_null() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c
        .send(json!({"id": 2, "method": "events.append", "params": {
            "kind": "shell_cmd", "source": "shell",
            "payload": {"cmd": "rat emit foo", "cwd": "/tmp"}
        }}))
        .await;
    assert!(resp.error.is_none());
    // Some(Null) round-trips to None through Option<Value>; both mean "dropped"
    assert!(resp.result.unwrap_or(serde_json::Value::Null).is_null());
}

#[tokio::test]
async fn invalid_json_returns_invalid_request() {
    let (_tmp, socket) = start().await;
    let s = UnixStream::connect(&socket).await.unwrap();
    let (r, mut w) = s.into_split();
    w.write_all(b"this is not json\n").await.unwrap();
    let mut lines = BufReader::new(r).lines();
    let line = lines.next_line().await.unwrap().unwrap();
    let resp: Response = serde_json::from_str(&line).unwrap();
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c.send(json!({"id": 2, "method": "nope"})).await;
    assert_eq!(resp.error.unwrap().code, errcodes::METHOD_NOT_FOUND);
}

#[tokio::test]
async fn workbench_runs_empty() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c.send(json!({"id": 2, "method": "workbench.runs", "params": {}})).await;
    assert!(resp.error.is_none());
    let runs = resp.result.unwrap();
    assert!(runs.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn approvals_pending_empty() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c.send(json!({"id": 2, "method": "approvals.pending"})).await;
    assert!(resp.error.is_none());
    let approvals = resp.result.unwrap();
    assert!(approvals.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn pins_rpc_pin_recent_list_unpin_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("ratd.sock");
    let db = tmp.path().join("rato.db");
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let store = Store::open(&db, clock.clone()).unwrap();
    let ingest = Arc::new(Ingest::new(store.clone(), clock.clone(), Sessionizer::new(DEFAULT_GAP_MS)));
    let mode = Arc::new(ModeManager::new(0));
    let task_runner = TaskRunner::new(store.clone(), Tmux::new(unique_tmux_socket()), clock.clone());
    let ring_key = Arc::new(RingKey::ephemeral());
    let ring = Arc::new(RingWriter {
        dir: tmp.path().join("ring"),
        segment_secs: 10,
        ttl_secs: 1_200,
        clock: clock.clone(),
    });
    ring.write_segment(Media::Screen, b"captured jpeg bytes", &ring_key).unwrap();
    let pins = PinService::new(
        store.clone(),
        ring,
        ring_key,
        Arc::new(StaticPinKeyStore([3u8; 32])),
        tmp.path().join("pins"),
        clock,
    );
    let ctx = Arc::new(ServerCtx {
        store,
        ingest,
        mode,
        started: Instant::now(),
        db_path: db,
        clock: Arc::new(SystemClock),
        embedder: None,
        llm_status: LlmStatusState::disabled(),
        task_runner,
        pins: Some(pins),
    });
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(serve(listener, ctx));

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let pinned = c
        .send(json!({"id": 2, "method": "pins.pin_recent", "params": {"media": "screen", "minutes": 5}}))
        .await;
    assert!(pinned.error.is_none(), "pin_recent should succeed: {:?}", pinned.error);
    let pin = pinned.result.unwrap();
    let pin_id = pin["id"].as_str().unwrap().to_string();
    let pin_path = std::path::PathBuf::from(pin["path"].as_str().unwrap());
    assert!(pin_path.is_dir());

    let listed = c.send(json!({"id": 3, "method": "pins.list"})).await.result.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);

    let unpinned = c
        .send(json!({"id": 4, "method": "pins.unpin", "params": {"id": pin_id}}))
        .await;
    assert!(unpinned.error.is_none(), "unpin should succeed: {:?}", unpinned.error);
    assert!(!pin_path.exists());
    let listed = c.send(json!({"id": 5, "method": "pins.list"})).await.result.unwrap();
    assert!(listed.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn approvals_decide_not_found() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c.send(json!({
        "id": 2, "method": "approvals.decide",
        "params": {"id": "nonexistent", "verdict": "approve"}
    })).await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn approvals_decide_bad_verdict() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c.send(json!({
        "id": 2, "method": "approvals.decide",
        "params": {"id": "x", "verdict": "maybe"}
    })).await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn workbench_start_unknown_adapter() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c.send(json!({
        "id": 2, "method": "workbench.start",
        "params": {"project_id": "proj1", "title": "test", "adapter": "badagent"}
    })).await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn workbench_start_unknown_project() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c.send(json!({
        "id": 2, "method": "workbench.start",
        "params": {"project_id": "nonexistent-id", "title": "test", "adapter": "fakeagent"}
    })).await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn workbench_tail_not_found() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c.send(json!({
        "id": 2, "method": "workbench.tail",
        "params": {"run_id": "nonexistent"}
    })).await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn approvals_decide_deny_seeded() {
    // Seed a pending approval in the store, then deny it via RPC.
    let (_tmp, socket, store) = start_with_store().await;
    let clock: Arc<dyn rat_core::clock::Clock> = Arc::new(SystemClock);
    let approval = store.insert_approval(rat_store::rows::NewApproval {
        kind: "merge_back".into(),
        risk: 2,
        title: "test approval".into(),
        reason: "testing".into(),
        cwd: None,
        target: None,
        agent_identity: "fakeagent".into(),
        payload: serde_json::json!({}),
        expected_impact: serde_json::json!({}),
        expires_at: clock.now_ms() + 3_600_000,
    }).await.unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;

    // First check pending shows 1
    let resp = c.send(json!({"id": 2, "method": "approvals.pending"})).await;
    let arr = resp.result.unwrap();
    assert_eq!(arr.as_array().unwrap().len(), 1);

    // Deny it
    let resp = c.send(json!({
        "id": 3, "method": "approvals.decide",
        "params": {"id": approval.id, "verdict": "deny", "note": "no thanks"}
    })).await;
    assert!(resp.error.is_none(), "deny should succeed: {:?}", resp.error);
    let decided = resp.result.unwrap();
    assert_eq!(decided["status"], "denied");

    // Now pending should be empty
    let resp = c.send(json!({"id": 4, "method": "approvals.pending"})).await;
    let arr = resp.result.unwrap();
    assert!(arr.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn r3_approval_requires_slug() {
    // Seed an R3 approval (risk=3), try to approve without slug → error.
    let (_tmp, socket, store) = start_with_store().await;
    let clock: Arc<dyn rat_core::clock::Clock> = Arc::new(SystemClock);
    let approval = store.insert_approval(rat_store::rows::NewApproval {
        kind: "global_install".into(),
        risk: 3,
        title: "R3 test".into(),
        reason: "testing slug gate".into(),
        cwd: None,
        target: None,
        agent_identity: "test".into(),
        payload: serde_json::json!({}),
        expected_impact: serde_json::json!({}),
        expires_at: clock.now_ms() + 3_600_000,
    }).await.unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;

    // Approve without slug → error
    let resp = c.send(json!({
        "id": 2, "method": "approvals.decide",
        "params": {"id": approval.id, "verdict": "approve"}
    })).await;
    assert!(resp.error.is_some(), "R3 without slug must be rejected");
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);

    // Approve with wrong slug → error
    let resp = c.send(json!({
        "id": 3, "method": "approvals.decide",
        "params": {"id": approval.id, "verdict": "approve", "slug": "wrong1"}
    })).await;
    assert!(resp.error.is_some(), "R3 with wrong slug must be rejected");

    // Approve with correct slug → succeeds
    let correct_slug: String = approval.id.chars().rev().take(6).collect::<String>().chars().rev().collect();
    let resp = c.send(json!({
        "id": 4, "method": "approvals.decide",
        "params": {"id": approval.id, "verdict": "approve", "slug": correct_slug}
    })).await;
    // This may fail because the approval kind is not "merge_back" — execute_merge won't be called.
    // But the slug gate should pass and the decision itself should succeed.
    assert!(resp.error.is_none(), "R3 with correct slug should proceed: {:?}", resp.error);
    let decided = resp.result.unwrap();
    assert_eq!(decided["status"], "approved");
}

// ---------------------------------------------------------------------------
// workbench.merge_back RPC integration test
//
// Guards: skips when git is not on PATH.
// Uses a unique tmux socket and cleans up on drop.
// Seeds a "done" AgentRun (real git worktree + commit) then calls the RPC.
// ---------------------------------------------------------------------------

fn has_binary_rpc(name: &str) -> bool {
    std::process::Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git_rpc(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git failed");
    assert!(status.success(), "git {:?} failed in {}", args, dir.display());
}

async fn start_with_unique_socket(
    tmux_socket: &str,
    store: Store,
    clock: Arc<FakeClock>,
) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("ratd.sock");
    let db_path = tmp.path().join("ignored.db"); // store already opened
    let clock_arc: Arc<dyn rat_core::clock::Clock> = clock.clone();
    let ingest = Arc::new(Ingest::new(
        store.clone(),
        clock_arc.clone(),
        Sessionizer::new(DEFAULT_GAP_MS),
    ));
    let mode = Arc::new(ModeManager::new(0));
    let task_runner = TaskRunner::new(
        store.clone(),
        Tmux::new(tmux_socket.to_string()),
        clock_arc.clone(),
    );
    let ctx = Arc::new(ServerCtx {
        store,
        ingest,
        mode,
        started: Instant::now(),
        db_path,
        clock: clock_arc,
        embedder: None,
        llm_status: LlmStatusState::disabled(),
        task_runner,
        pins: None,
    });
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(serve(listener, ctx));
    (tmp, socket)
}

#[tokio::test]
async fn workbench_merge_back_rpc_returns_approval() {
    if !has_binary_rpc("git") {
        eprintln!("skipping workbench_merge_back_rpc_returns_approval: git not found");
        return;
    }

    // Unique tmux socket so teardown doesn't affect other tests.
    let tmux_socket = unique_tmux_socket();
    let tmux = Tmux::new(tmux_socket.clone());
    struct TmuxGuard(Tmux);
    impl Drop for TmuxGuard {
        fn drop(&mut self) {
            let _ = self.0.kill_server();
        }
    }
    let _tmux_guard = TmuxGuard(tmux.clone());

    // Set up a real git repo with an initial commit.
    let tmp_repo = tempfile::TempDir::new().unwrap();
    let repo = tmp_repo.path().to_path_buf();
    git_rpc(&repo, &["init", "-b", "main"]);
    git_rpc(&repo, &["config", "user.email", "test@test.local"]);
    git_rpc(&repo, &["config", "user.name", "Test"]);
    std::fs::write(repo.join("README.md"), "# rpc test\n").unwrap();
    git_rpc(&repo, &["add", "README.md"]);
    git_rpc(&repo, &["commit", "-m", "init"]);

    // Open store and create project.
    let tmp_store = tempfile::TempDir::new().unwrap();
    let clock = FakeClock::at(5_000_000);
    let store = Store::open(&tmp_store.path().join("test.db"), clock.clone()).unwrap();

    let project = store
        .upsert_project(repo.display().to_string(), "rpc-merge-test".into())
        .await
        .expect("upsert project");

    // Create a worktree branch and make a commit inside it.
    let wt = rat_workbench::worktree::create(&repo, "rpctest", "rpc-merge-branch", "HEAD")
        .expect("create worktree");
    git_rpc(&wt.path, &["config", "user.email", "test@test.local"]);
    git_rpc(&wt.path, &["config", "user.name", "Test"]);
    std::fs::write(wt.path.join("AGENT_NOTE.md"), "rpc test\n").unwrap();
    git_rpc(&wt.path, &["add", "AGENT_NOTE.md"]);
    git_rpc(&wt.path, &["commit", "-m", "rpc: agent change"]);

    // Insert a "done" AgentRun pointing at the worktree.
    let run = store
        .insert_agent_run(rat_store::rows::NewAgentRun {
            adapter: "fakeagent".into(),
            task_title: "rpc merge back test".into(),
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

    // Start the daemon server.
    let (_tmp_srv, socket) =
        start_with_unique_socket(&tmux_socket, store.clone(), clock.clone()).await;

    // Connect and handshake.
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;

    // Call workbench.merge_back.
    let resp = c
        .send(json!({
            "id": 2,
            "method": "workbench.merge_back",
            "params": {"run_id": run.id}
        }))
        .await;
    assert!(resp.error.is_none(), "merge_back must succeed: {:?}", resp.error);

    let approval = resp.result.unwrap();
    assert_eq!(approval["kind"], "merge_back", "approval kind must be merge_back");
    assert_eq!(approval["risk"], 2, "MergeBack must be R2");
    assert_eq!(approval["status"], "pending", "new approval must be pending");

    // The approval must now appear in approvals.pending.
    let pending_resp = c.send(json!({"id": 3, "method": "approvals.pending"})).await;
    assert!(pending_resp.error.is_none());
    let pending = pending_resp.result.unwrap();
    let arr = pending.as_array().unwrap();
    assert!(!arr.is_empty(), "approvals.pending must contain the new merge_back approval");
    assert!(
        arr.iter().any(|a| a["kind"] == "merge_back"),
        "approvals.pending must include the merge_back approval"
    );
}

#[tokio::test]
async fn workbench_merge_back_rpc_unknown_run_returns_error() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c
        .send(json!({
            "id": 2,
            "method": "workbench.merge_back",
            "params": {"run_id": "nonexistent-run-id"}
        }))
        .await;
    assert!(resp.error.is_some(), "unknown run_id must return an error");
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

// ---------------------------------------------------------------------------
// workbench.runs poll-on-read integration test
//
// Guards: skips when git or tmux not found on PATH.
// Starts a real fakeagent task via workbench.start, then retries
// workbench.runs until the run's status becomes "done". This proves that
// poll-on-read (inside the WORKBENCH_RUNS arm) advances running → done
// without waiting for the 3s background sweep.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workbench_runs_poll_on_read_advances_to_done() {
    if !has_binary_rpc("git") || !has_binary_rpc("tmux") {
        eprintln!("skipping workbench_runs_poll_on_read_advances_to_done: git or tmux not found");
        return;
    }

    // Unique tmux socket so this test does not affect any running tmux server.
    let tmux_socket = unique_tmux_socket();
    let tmux = Tmux::new(tmux_socket.clone());
    struct TmuxGuard(Tmux);
    impl Drop for TmuxGuard {
        fn drop(&mut self) {
            let _ = self.0.kill_server();
        }
    }
    let _tmux_guard = TmuxGuard(tmux.clone());

    // Set up a real git repo with an initial commit.
    let tmp_repo = tempfile::TempDir::new().unwrap();
    let repo = tmp_repo.path().to_path_buf();
    git_rpc(&repo, &["init", "-b", "main"]);
    git_rpc(&repo, &["config", "user.email", "test@test.local"]);
    git_rpc(&repo, &["config", "user.name", "Test"]);
    std::fs::write(repo.join("README.md"), "# poll-on-read test\n").unwrap();
    git_rpc(&repo, &["add", "README.md"]);
    git_rpc(&repo, &["commit", "-m", "init"]);

    // Open store and register the project.
    let tmp_store = tempfile::TempDir::new().unwrap();
    let clock = FakeClock::at(6_000_000);
    let store = Store::open(&tmp_store.path().join("test.db"), clock.clone()).unwrap();
    let project = store
        .upsert_project(repo.display().to_string(), "poll-on-read-test".into())
        .await
        .expect("upsert project");

    // Start the daemon server with the unique tmux socket.
    let (_tmp_srv, socket) =
        start_with_unique_socket(&tmux_socket, store.clone(), clock.clone()).await;

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;

    // Start a fakeagent run via RPC.
    let start_resp = c
        .send(json!({
            "id": 2,
            "method": "workbench.start",
            "params": {
                "project_id": project.id,
                "title": "poll-on-read task",
                "adapter": "fakeagent"
            }
        }))
        .await;
    assert!(start_resp.error.is_none(), "workbench.start must succeed: {:?}", start_resp.error);
    let run_id = start_resp.result.unwrap()["id"].as_str().unwrap().to_string();
    assert!(!run_id.is_empty());

    // Retry workbench.runs until the run's status becomes "done" (or we time out).
    // Each call triggers poll-on-read which calls TaskRunner::poll internally.
    let mut final_status = String::new();
    for req_id in (3u32..).take(40) {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let resp = c
            .send(json!({
                "id": req_id,
                "method": "workbench.runs",
                "params": {"n": 10}
            }))
            .await;
        assert!(resp.error.is_none(), "workbench.runs must not error: {:?}", resp.error);
        let runs = resp.result.unwrap();
        if let Some(run) = runs.as_array().unwrap().iter().find(|r| r["id"] == run_id) {
            let status = run["status"].as_str().unwrap_or("").to_string();
            if status == "done" || status == "failed" {
                final_status = status;
                break;
            }
        }
    }

    assert_eq!(
        final_status, "done",
        "run {run_id} should reach status 'done' via poll-on-read, got: '{final_status}'"
    );
}
