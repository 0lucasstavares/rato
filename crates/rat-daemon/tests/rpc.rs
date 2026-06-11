use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rat_core::clock::SystemClock;
use rat_daemon::server::{serve, ServerCtx};
use rat_proto::{errcodes, Response, PROTO_VERSION};
use rat_store::store::Store;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

async fn start() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("ratd.sock");
    let db = tmp.path().join("rato.db");
    let store = Store::open(&db, Arc::new(SystemClock)).unwrap();
    let ctx = Arc::new(ServerCtx { store, started: Instant::now(), db_path: db });
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(serve(listener, ctx));
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
