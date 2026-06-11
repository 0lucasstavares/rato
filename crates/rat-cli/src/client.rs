use std::path::Path;

use anyhow::Context;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

use rat_proto::{methods, HelloParams, HelloResult, Request, Response, StatusResult, PROTO_VERSION};

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
            "protocol mismatch: daemon v{}, rat v{}",
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
