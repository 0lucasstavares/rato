use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rat_core::clock::SystemClock;
use rat_daemon::ingest::Ingest;
use rat_daemon::mode::ModeManager;
use rat_daemon::server::{LlmStatusState, serve, ServerCtx};
use rat_daemon::sessionizer::{Sessionizer, DEFAULT_GAP_MS};
use rat_proto::{errcodes, Response, PROTO_VERSION};
use rat_store::store::Store;
use rat_workbench::runner::TaskRunner;
use rat_workbench::tmux::Tmux;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

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
