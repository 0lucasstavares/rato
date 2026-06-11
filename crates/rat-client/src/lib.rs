use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

use rat_proto::{methods, HelloParams, HelloResult, Request, Response, StatusResult, PROTO_VERSION};

/// One NDJSON-RPC connection to ratd, hello handshake included.
pub struct Client {
    lines: Lines<BufReader<OwnedReadHalf>>,
    w: OwnedWriteHalf,
    next_id: u64,
}

impl Client {
    pub async fn connect(socket: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket)
            .await
            .with_context(|| format!("connecting to {} (is ratd running?)", socket.display()))?;
        let (r, w) = stream.into_split();
        let mut client = Self { lines: BufReader::new(r).lines(), w, next_id: 0 };
        let hello: HelloResult = serde_json::from_value(
            client
                .call(
                    methods::HELLO,
                    serde_json::to_value(HelloParams { proto_version: PROTO_VERSION })?,
                )
                .await?,
        )?;
        anyhow::ensure!(
            hello.proto_version == PROTO_VERSION,
            "protocol mismatch: daemon v{}, client v{}",
            hello.proto_version,
            PROTO_VERSION
        );
        Ok(client)
    }

    pub async fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.next_id += 1;
        let req = Request { id: self.next_id, method: method.to_string(), params };
        let mut buf = serde_json::to_vec(&req)?;
        buf.push(b'\n');
        self.w.write_all(&buf).await?;
        let line = self.lines.next_line().await?.context("daemon closed the connection")?;
        let resp: Response = serde_json::from_str(&line)?;
        if let Some(err) = resp.error {
            anyhow::bail!("rpc error {}: {}", err.code, err.message);
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    pub async fn status(&mut self) -> anyhow::Result<StatusResult> {
        Ok(serde_json::from_value(self.call(methods::STATUS, json!({})).await?)?)
    }
}

/// Persistent connection that transparently reconnects (and re-hellos) after
/// IO errors — the shell uses one of these for its whole lifetime.
pub struct ManagedClient {
    socket: PathBuf,
    inner: Option<Client>,
}

impl ManagedClient {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket, inner: None }
    }

    /// True when a live (or freshly re-established) connection exists.
    pub async fn ensure_connected(&mut self) -> bool {
        if self.inner.is_none() {
            self.inner = Client::connect(&self.socket).await.ok();
        }
        self.inner.is_some()
    }

    pub async fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        if self.inner.is_none() {
            self.inner = Some(Client::connect(&self.socket).await?);
        }
        let client = self.inner.as_mut().expect("just connected");
        match client.call(method, params.clone()).await {
            Ok(v) => Ok(v),
            Err(first_err) => {
                // connection may be stale (daemon restarted): one reconnect attempt
                self.inner = None;
                match Client::connect(&self.socket).await {
                    Ok(mut fresh) => {
                        let v = fresh.call(method, params).await?;
                        self.inner = Some(fresh);
                        Ok(v)
                    }
                    Err(_) => Err(first_err),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    use rat_daemon::ingest::Ingest;
    use rat_daemon::mode::ModeManager;
    use rat_daemon::server::{LlmStatusState, serve, ServerCtx};
    use rat_daemon::sessionizer::{Sessionizer, DEFAULT_GAP_MS};
    use rat_store::store::Store;
    use rat_workbench::runner::TaskRunner;
    use rat_workbench::tmux::Tmux;

    /// A daemon in its own runtime on its own thread — dropping the runtime
    /// (via the shutdown signal) kills connection handler tasks too, unlike
    /// aborting just the accept loop.
    struct TestDaemon {
        stop: Option<tokio::sync::oneshot::Sender<()>>,
        joined: Option<std::thread::JoinHandle<()>>,
    }

    impl TestDaemon {
        fn start(socket: PathBuf, db: PathBuf) -> Self {
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
            let joined = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                rt.block_on(async move {
                    let clock: Arc<dyn rat_core::clock::Clock> =
                        Arc::new(rat_core::clock::SystemClock);
                    let store = Store::open(&db, clock.clone()).unwrap();
                    let ingest = Arc::new(Ingest::new(
                        store.clone(),
                        clock.clone(),
                        Sessionizer::new(DEFAULT_GAP_MS),
                    ));
                    let mode = Arc::new(ModeManager::new(0));
                    let task_runner = TaskRunner::new(store.clone(), Tmux::new(format!("rato-test-{}", std::process::id())), clock.clone());
                    let ctx = Arc::new(ServerCtx {
                        store,
                        ingest,
                        mode,
                        started: Instant::now(),
                        db_path: db,
                        clock,
                        embedder: None,
                        llm_status: LlmStatusState::disabled(),
                        task_runner,
                    });
                    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
                    ready_tx.send(()).unwrap();
                    tokio::select! {
                        _ = serve(listener, ctx) => {}
                        _ = stopped => {}
                    }
                });
                // runtime drops here, killing all connection tasks
            });
            ready_rx.recv().unwrap();
            Self { stop: Some(stop), joined: Some(joined) }
        }

        fn shutdown(mut self) {
            let _ = self.stop.take().unwrap().send(());
            self.joined.take().unwrap().join().unwrap();
        }
    }

    #[tokio::test]
    async fn managed_client_survives_daemon_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("s.sock");
        let db = tmp.path().join("d.db");

        let daemon = TestDaemon::start(socket.clone(), db.clone());
        let mut mc = ManagedClient::new(socket.clone());
        assert!(mc.call(methods::STATUS, Value::Null).await.is_ok());

        // kill the daemon, drop the socket
        daemon.shutdown();
        std::fs::remove_file(&socket).unwrap();
        assert!(mc.call(methods::STATUS, Value::Null).await.is_err());

        // bring it back: same ManagedClient reconnects on its own
        let _daemon2 = TestDaemon::start(socket.clone(), db);
        let v = mc.call(methods::STATUS, Value::Null).await.unwrap();
        assert!(v["version"].is_string());
    }

    #[tokio::test]
    async fn test_pushbacks_recent_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("s.sock");
        let db = tmp.path().join("d.db");

        let _daemon = TestDaemon::start(socket.clone(), db);
        let mut c = Client::connect(&socket).await.unwrap();

        let pbs: Vec<rat_proto::PushbackDto> = serde_json::from_value(
            c.call(rat_proto::methods::PUSHBACKS_RECENT, Value::Null).await.unwrap(),
        )
        .unwrap();
        assert!(pbs.is_empty());
    }

    #[tokio::test]
    async fn test_memory_search_no_results() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("s.sock");
        let db = tmp.path().join("d.db");

        let _daemon = TestDaemon::start(socket.clone(), db);
        let mut c = Client::connect(&socket).await.unwrap();

        let hits: Vec<rat_proto::HitDto> = serde_json::from_value(
            c.call(
                rat_proto::methods::MEMORY_SEARCH,
                serde_json::json!({ "query": "cargo", "n": 5 }),
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn test_llm_status() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("s.sock");
        let db = tmp.path().join("d.db");

        let _daemon = TestDaemon::start(socket.clone(), db);
        let mut c = Client::connect(&socket).await.unwrap();

        let s: rat_proto::LlmStatusResult = serde_json::from_value(
            c.call(rat_proto::methods::LLM_STATUS, Value::Null).await.unwrap(),
        )
        .unwrap();
        // With disabled LlmStatusState: provider="openai", critic_enabled=false
        assert_eq!(s.provider, "openai");
        assert!(!s.critic_enabled);
        assert!(!s.embedding_enabled);
        assert!(s.last_error.is_none());
    }

    #[tokio::test]
    async fn test_pushbacks_feedback_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("s.sock");
        let db = tmp.path().join("d.db");

        let _daemon = TestDaemon::start(socket.clone(), db);
        let mut c = Client::connect(&socket).await.unwrap();

        let result = c
            .call(
                rat_proto::methods::PUSHBACKS_FEEDBACK,
                serde_json::json!({ "id": "nonexistent", "verdict": "useful" }),
            )
            .await;
        // Should return an RPC error (pushback not found)
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("pushback not found") || err.contains("-32600"));
    }
}
