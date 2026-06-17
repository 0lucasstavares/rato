use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rat_core::clock::{Clock, FakeClock, SystemClock};
use rat_daemon::ingest::Ingest;
use rat_daemon::mode::ModeManager;
use rat_daemon::pins::{PinKeyStore, PinService};
use rat_daemon::server::{serve, LlmStatusState, ServerCtx};
use rat_daemon::sessionizer::{Sessionizer, DEFAULT_GAP_MS};
use rat_proto::{errcodes, Response, PROTO_VERSION};
use rat_ring::{Media, RingKey, RingWriter};
use rat_store::rows::{NewDotfileEdit, NewTerminal};
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
    let ingest = Arc::new(Ingest::new(
        store.clone(),
        clock.clone(),
        Sessionizer::new(DEFAULT_GAP_MS),
    ));
    let mode = Arc::new(ModeManager::new(0));
    let task_runner = TaskRunner::new(
        store.clone(),
        Tmux::new(format!("rato-test-{}", std::process::id())),
        clock.clone(),
    );
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
        sensors: Arc::new(rat_daemon::sensors_health::SensorGate::new()),
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
        Self {
            lines: BufReader::new(r).lines(),
            w,
        }
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

    let recent = c
        .send(json!({"id": 4, "method": "events.recent", "params": {"limit": 10}}))
        .await;
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
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    let appended = c
        .send(json!({"id": 2, "method": "events.append", "params": {
            "kind": "shell_cmd", "source": "shell",
            "payload": {"cmd": "cargo build", "cwd": repo.to_string_lossy(), "exit": 0, "duration_ms": 900}
        }}))
        .await;
    assert!(appended.result.unwrap()["project_id"].is_string());

    let projects = c
        .send(json!({"id": 3, "method": "projects.list"}))
        .await
        .result
        .unwrap();
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["name"], "acme");

    let sessions = c
        .send(json!({"id": 4, "method": "sessions.recent"}))
        .await
        .result
        .unwrap();
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

    let mode = c
        .send(json!({"id": 6, "method": "mode.get"}))
        .await
        .result
        .unwrap();
    assert_eq!(mode["mode"], "active");
}

#[tokio::test]
async fn rat_emit_loop_guard_returns_null() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
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
async fn terminals_list_and_set_role_round_trip() {
    let (_tmp, socket, store) = start_with_store().await;
    let terminal = store
        .upsert_terminal(NewTerminal {
            tty: "/dev/pts/7".into(),
            pid: 42,
            emulator: "kitty".into(),
            tmux_target: Some("sess:0.1".into()),
            role: "foreign".into(),
            project_id: None,
            cmd_hash: "hash1".into(),
            meta: json!({"adapter": "claude"}),
        })
        .await
        .unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    let listed = c
        .send(json!({"id": 2, "method": "terminals.list"}))
        .await
        .result
        .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["role"], "foreign");

    let updated = c
        .send(json!({
            "id": 3,
            "method": "terminals.set_role",
            "params": {"id": terminal.id, "role": "operator"}
        }))
        .await
        .result
        .unwrap();
    assert_eq!(updated["role"], "operator");
    assert_eq!(updated["tty"], "/dev/pts/7");
}

#[tokio::test]
async fn dotfile_edit_revert_restores_bytes_and_links_audit_rows() {
    let (tmp, socket, store) = start_with_store().await;
    let path = tmp.path().join("settings.json");
    let before = b"{\"old\":true}\n";
    let after = b"{\"old\":false}\n";
    std::fs::write(&path, after).unwrap();

    let before_blob = store.insert_blob(before.to_vec(), 1_000).await.unwrap();
    let after_blob = store.insert_blob(after.to_vec(), 1_001).await.unwrap();
    let edit = store
        .insert_dotfile_edit(NewDotfileEdit {
            path: path.display().to_string(),
            kind: "json".into(),
            before_blob: before_blob.id,
            after_blob: after_blob.id,
            diff: "--- before\n+++ after\n".into(),
            reason: "test edit".into(),
            source: "rat-dotfile".into(),
            risk: 2,
            applied: true,
            meta: json!({}),
        })
        .await
        .unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    let reverted = c
        .send(json!({
            "id": 2,
            "method": "dotfile_edits.revert",
            "params": {"id": edit.id}
        }))
        .await
        .result
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(reverted["path"], path.display().to_string());
    assert_eq!(reverted["meta"]["reverts"], edit.id);

    let original = store.get_dotfile_edit(edit.id).await.unwrap().unwrap();
    assert_eq!(original.reverted_by.as_deref(), reverted["id"].as_str());
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
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c
        .send(json!({"id": 2, "method": "workbench.runs", "params": {}}))
        .await;
    assert!(resp.error.is_none());
    let runs = resp.result.unwrap();
    assert!(runs.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn approvals_pending_empty() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c
        .send(json!({"id": 2, "method": "approvals.pending"}))
        .await;
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
    let ingest = Arc::new(Ingest::new(
        store.clone(),
        clock.clone(),
        Sessionizer::new(DEFAULT_GAP_MS),
    ));
    let mode = Arc::new(ModeManager::new(0));
    let task_runner = TaskRunner::new(
        store.clone(),
        Tmux::new(unique_tmux_socket()),
        clock.clone(),
    );
    let ring_key = Arc::new(RingKey::ephemeral());
    let ring = Arc::new(RingWriter {
        dir: tmp.path().join("ring"),
        segment_secs: 10,
        ttl_secs: 1_200,
        clock: clock.clone(),
    });
    ring.write_segment(Media::Screen, b"captured jpeg bytes", &ring_key)
        .unwrap();
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
        sensors: Arc::new(rat_daemon::sensors_health::SensorGate::new()),
    });
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(serve(listener, ctx));

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let pinned = c
        .send(json!({"id": 2, "method": "pins.pin_recent", "params": {"media": "screen", "minutes": 5}}))
        .await;
    assert!(
        pinned.error.is_none(),
        "pin_recent should succeed: {:?}",
        pinned.error
    );
    let pin = pinned.result.unwrap();
    let pin_id = pin["id"].as_str().unwrap().to_string();
    let pin_path = std::path::PathBuf::from(pin["path"].as_str().unwrap());
    assert!(pin_path.is_dir());

    let listed = c
        .send(json!({"id": 3, "method": "pins.list"}))
        .await
        .result
        .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);

    let ring_status = c
        .send(json!({"id": 30, "method": "ring.status"}))
        .await
        .result
        .unwrap();
    let ring_rows = ring_status.as_array().unwrap();
    let screen = ring_rows
        .iter()
        .find(|row| row["media"].as_str() == Some("screen"))
        .expect("screen ring status");
    assert_eq!(screen["segment_count"].as_u64(), Some(1));

    let unpinned = c
        .send(json!({"id": 4, "method": "pins.unpin", "params": {"id": pin_id}}))
        .await;
    assert!(
        unpinned.error.is_none(),
        "unpin should succeed: {:?}",
        unpinned.error
    );
    assert!(!pin_path.exists());
    let listed = c
        .send(json!({"id": 5, "method": "pins.list"}))
        .await
        .result
        .unwrap();
    assert!(listed.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn approvals_decide_not_found() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c
        .send(json!({
            "id": 2, "method": "approvals.decide",
            "params": {"id": "nonexistent", "verdict": "approve"}
        }))
        .await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn approvals_decide_bad_verdict() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c
        .send(json!({
            "id": 2, "method": "approvals.decide",
            "params": {"id": "x", "verdict": "maybe"}
        }))
        .await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn workbench_start_unknown_adapter() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c
        .send(json!({
            "id": 2, "method": "workbench.start",
            "params": {"project_id": "proj1", "title": "test", "adapter": "badagent"}
        }))
        .await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn workbench_start_unknown_project() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c
        .send(json!({
            "id": 2, "method": "workbench.start",
            "params": {"project_id": "nonexistent-id", "title": "test", "adapter": "fakeagent"}
        }))
        .await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn workbench_start_docker_requires_image() {
    let (_tmp, socket, store) = start_with_store().await;
    let project = store
        .upsert_project("/tmp/rato-docker-test".into(), "rato-docker-test".into())
        .await
        .unwrap();
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c
        .send(json!({
            "id": 2,
            "method": "workbench.start",
            "params": {
                "project_id": project.id,
                "title": "test",
                "adapter": "fakeagent",
                "executor": "docker"
            }
        }))
        .await;
    assert!(resp.error.is_some());
    let err = resp.error.unwrap();
    assert_eq!(err.code, errcodes::INVALID_REQUEST);
    assert!(err.message.contains("docker_image"));
}

#[tokio::test]
async fn workbench_start_rejects_unknown_executor() {
    let (_tmp, socket, store) = start_with_store().await;
    let project = store
        .upsert_project("/tmp/rato-docker-test".into(), "rato-docker-test".into())
        .await
        .unwrap();
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c
        .send(json!({
            "id": 2,
            "method": "workbench.start",
            "params": {
                "project_id": project.id,
                "title": "test",
                "adapter": "fakeagent",
                "executor": "podman"
            }
        }))
        .await;
    assert!(resp.error.is_some());
    let err = resp.error.unwrap();
    assert_eq!(err.code, errcodes::INVALID_REQUEST);
    assert!(err.message.contains("local|docker"));
}

#[tokio::test]
async fn workbench_tail_not_found() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    let resp = c
        .send(json!({
            "id": 2, "method": "workbench.tail",
            "params": {"run_id": "nonexistent"}
        }))
        .await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn approvals_decide_deny_seeded() {
    // Seed a pending approval in the store, then deny it via RPC.
    let (_tmp, socket, store) = start_with_store().await;
    let clock: Arc<dyn rat_core::clock::Clock> = Arc::new(SystemClock);
    let approval = store
        .insert_approval(rat_store::rows::NewApproval {
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
        })
        .await
        .unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    // First check pending shows 1
    let resp = c
        .send(json!({"id": 2, "method": "approvals.pending"}))
        .await;
    let arr = resp.result.unwrap();
    assert_eq!(arr.as_array().unwrap().len(), 1);
    assert!(arr[0]["spoken_slug"].as_str().unwrap().contains('-'));

    // Deny it
    let resp = c
        .send(json!({
            "id": 3, "method": "approvals.decide",
            "params": {"id": approval.id, "verdict": "deny", "note": "no thanks"}
        }))
        .await;
    assert!(
        resp.error.is_none(),
        "deny should succeed: {:?}",
        resp.error
    );
    let decided = resp.result.unwrap();
    assert_eq!(decided["status"], "denied");

    // Now pending should be empty
    let resp = c
        .send(json!({"id": 4, "method": "approvals.pending"}))
        .await;
    let arr = resp.result.unwrap();
    assert!(arr.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn voice_status_and_utterances_rpc() {
    let (_tmp, socket, store) = start_with_store().await;
    store
        .insert_voice_utterance(rat_store::rows::NewVoiceUtterance {
            lang: "pt".into(),
            text: "pina isso".into(),
            intent: Some("pin_recent".into()),
            wake_word: "ei rato".into(),
            handled: true,
        })
        .await
        .unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    let status = c.send(json!({"id": 2, "method": "voice.status"})).await;
    assert!(
        status.error.is_none(),
        "voice.status should succeed: {:?}",
        status.error
    );
    let status = status.result.unwrap();
    assert_eq!(status["enabled"], false);
    assert_eq!(status["prewake_ring_secs"], 8);
    assert!(status["backends"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["name"] == "mic"));

    let utterances = c
        .send(json!({"id": 3, "method": "voice.utterances", "params": {"limit": 5}}))
        .await;
    assert!(
        utterances.error.is_none(),
        "voice.utterances should succeed: {:?}",
        utterances.error
    );
    let arr = utterances.result.unwrap();
    assert_eq!(arr.as_array().unwrap().len(), 1);
    assert_eq!(arr[0]["lang"], "pt");
    assert_eq!(arr[0]["intent"], "pin_recent");
}

#[tokio::test]
async fn retention_status_rpc_returns_null_until_set_then_counts() {
    let (_tmp, socket, store) = start_with_store().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    let none = c.send(json!({"id": 2, "method": "retention.status"})).await;
    assert!(
        none.error.is_none(),
        "retention.status should succeed before first prune: {:?}",
        none.error
    );
    assert!(
        none.result.unwrap_or(serde_json::Value::Null).is_null(),
        "missing result and explicit null both represent no retention row"
    );

    store
        .set_retention_status(rat_store::rows::RetentionStatus {
            last_run_ms: 42_000,
            observations_deleted: 3,
            pins_expired: 1,
            api_calls_deleted: 2,
        })
        .await
        .unwrap();

    let set = c.send(json!({"id": 3, "method": "retention.status"})).await;
    assert!(
        set.error.is_none(),
        "retention.status should succeed after prune: {:?}",
        set.error
    );
    let status = set.result.unwrap();
    assert_eq!(status["last_run_ms"], 42_000);
    assert_eq!(status["observations_deleted"], 3);
    assert_eq!(status["pins_expired"], 1);
    assert_eq!(status["api_calls_deleted"], 2);
}

#[tokio::test]
async fn r3_approval_requires_slug() {
    // Seed an R3 approval (risk=3), try to approve without slug → error.
    let (_tmp, socket, store) = start_with_store().await;
    let clock: Arc<dyn rat_core::clock::Clock> = Arc::new(SystemClock);
    let approval = store
        .insert_approval(rat_store::rows::NewApproval {
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
        })
        .await
        .unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    // Approve without slug → error
    let resp = c
        .send(json!({
            "id": 2, "method": "approvals.decide",
            "params": {"id": approval.id, "verdict": "approve"}
        }))
        .await;
    assert!(resp.error.is_some(), "R3 without slug must be rejected");
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);

    // Approve with wrong slug → error
    let resp = c
        .send(json!({
            "id": 3, "method": "approvals.decide",
            "params": {"id": approval.id, "verdict": "approve", "slug": "wrong1"}
        }))
        .await;
    assert!(resp.error.is_some(), "R3 with wrong slug must be rejected");

    // Approve with correct slug → succeeds
    let correct_slug: String = approval
        .id
        .chars()
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let resp = c
        .send(json!({
            "id": 4, "method": "approvals.decide",
            "params": {"id": approval.id, "verdict": "approve", "slug": correct_slug}
        }))
        .await;
    // This may fail because the approval kind is not "merge_back" — execute_merge won't be called.
    // But the slug gate should pass and the decision itself should succeed.
    assert!(
        resp.error.is_none(),
        "R3 with correct slug should proceed: {:?}",
        resp.error
    );
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
    assert!(
        status.success(),
        "git {:?} failed in {}",
        args,
        dir.display()
    );
}

#[tokio::test]
async fn dotfile_edit_apply_validates_writes_and_audits() {
    let (tmp, socket, store) = start_with_store().await;
    let path = tmp.path().join("settings.toml");
    std::fs::write(&path, "old = true\n").unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    let applied = c
        .send(json!({
            "id": 2,
            "method": "dotfile_edits.apply",
            "params": {
                "path": path.display().to_string(),
                "kind": "toml",
                "contents": "old = false\n",
                "reason": "test managed edit",
                "source": "rpc-test",
                "risk": 2
            }
        }))
        .await;
    assert!(applied.error.is_none(), "{:?}", applied.error);
    let row = applied.result.unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old = false\n");
    assert_eq!(row["path"], path.display().to_string());
    assert_eq!(row["kind"], "toml");
    assert_eq!(row["source"], "rpc-test");
    assert_eq!(row["risk"], 2);

    let stored = store
        .get_dotfile_edit(row["id"].as_str().unwrap().to_string())
        .await
        .unwrap()
        .unwrap();
    let before_blob = store.get_blob(stored.before_blob).await.unwrap().unwrap();
    let after_blob = store.get_blob(stored.after_blob).await.unwrap().unwrap();
    assert_eq!(before_blob.bytes, b"old = true\n");
    assert_eq!(after_blob.bytes, b"old = false\n");
}

#[tokio::test]
async fn dotfile_edit_apply_rejects_invalid_config_before_write_or_audit() {
    let (tmp, socket, store) = start_with_store().await;
    let path = tmp.path().join("settings.toml");
    std::fs::write(&path, "old = true\n").unwrap();

    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    let rejected = c
        .send(json!({
            "id": 2,
            "method": "dotfile_edits.apply",
            "params": {
                "path": path.display().to_string(),
                "kind": "toml",
                "contents": "[broken\n",
                "reason": "bad edit"
            }
        }))
        .await;
    assert!(rejected.error.is_some());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old = true\n");
    assert!(store.recent_dotfile_edits(10).await.unwrap().is_empty());
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
        sensors: Arc::new(rat_daemon::sensors_health::SensorGate::new()),
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
        .update_agent_run_status(
            run.id.clone(),
            "done".into(),
            Some(clock.now_ms()),
            None,
            None,
        )
        .await
        .expect("update status");

    // Start the daemon server.
    let (_tmp_srv, socket) =
        start_with_unique_socket(&tmux_socket, store.clone(), clock.clone()).await;

    // Connect and handshake.
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

    // Call workbench.merge_back.
    let resp = c
        .send(json!({
            "id": 2,
            "method": "workbench.merge_back",
            "params": {"run_id": run.id}
        }))
        .await;
    assert!(
        resp.error.is_none(),
        "merge_back must succeed: {:?}",
        resp.error
    );

    let approval = resp.result.unwrap();
    assert_eq!(
        approval["kind"], "merge_back",
        "approval kind must be merge_back"
    );
    assert_eq!(approval["risk"], 2, "MergeBack must be R2");
    assert_eq!(
        approval["status"], "pending",
        "new approval must be pending"
    );

    // The approval must now appear in approvals.pending.
    let pending_resp = c
        .send(json!({"id": 3, "method": "approvals.pending"}))
        .await;
    assert!(pending_resp.error.is_none());
    let pending = pending_resp.result.unwrap();
    let arr = pending.as_array().unwrap();
    assert!(
        !arr.is_empty(),
        "approvals.pending must contain the new merge_back approval"
    );
    assert!(
        arr.iter().any(|a| a["kind"] == "merge_back"),
        "approvals.pending must include the merge_back approval"
    );
}

#[tokio::test]
async fn workbench_merge_back_rpc_unknown_run_returns_error() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
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
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;

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
    assert!(
        start_resp.error.is_none(),
        "workbench.start must succeed: {:?}",
        start_resp.error
    );
    let run_id = start_resp.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
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
        assert!(
            resp.error.is_none(),
            "workbench.runs must not error: {:?}",
            resp.error
        );
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
