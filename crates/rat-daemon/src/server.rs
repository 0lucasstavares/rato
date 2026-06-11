use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use rat_proto::{
    errcodes, methods, HelloParams, HelloResult, NewEvent, ObsRecentParams, RecentParams, Request,
    Response, StatusResult, PROTO_VERSION,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use rat_store::store::Store;

use crate::ingest::Ingest;
use crate::mode::ModeManager;

pub struct ServerCtx {
    pub store: Store,
    pub ingest: Arc<Ingest>,
    pub mode: Arc<ModeManager>,
    pub started: Instant,
    pub db_path: PathBuf,
}

/// Accept loop. Runs until the task is dropped.
pub async fn serve(listener: UnixListener, ctx: Arc<ServerCtx>) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(stream, ctx).await {
                        tracing::debug!("connection ended: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("accept error: {e}"),
        }
    }
}

async fn handle_conn(stream: UnixStream, ctx: Arc<ServerCtx>) -> std::io::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    let mut hello_done = false;
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let resp = dispatch(&line, &mut hello_done, &ctx).await;
        let mut buf = serde_json::to_vec(&resp).expect("response serializes");
        buf.push(b'\n');
        w.write_all(&buf).await?;
    }
    Ok(())
}

async fn dispatch(line: &str, hello_done: &mut bool, ctx: &ServerCtx) -> Response {
    let req: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            return Response::err(0, errcodes::INVALID_REQUEST, format!("invalid request: {e}"))
        }
    };
    match req.method.as_str() {
        methods::HELLO => {
            let params: HelloParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad hello params: {e}"),
                    )
                }
            };
            if params.proto_version != PROTO_VERSION {
                return Response::err(
                    req.id,
                    errcodes::PROTO_MISMATCH,
                    format!(
                        "daemon speaks proto v{PROTO_VERSION}, client sent v{}",
                        params.proto_version
                    ),
                );
            }
            *hello_done = true;
            let result = HelloResult {
                proto_version: PROTO_VERSION,
                server_version: env!("CARGO_PKG_VERSION").to_string(),
            };
            Response::ok(req.id, serde_json::to_value(result).expect("serializes"))
        }
        _ if !*hello_done => Response::err(req.id, errcodes::HELLO_REQUIRED, "hello required"),
        methods::STATUS => {
            let event_count = match ctx.store.count().await {
                Ok(n) => n,
                Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            };
            let result = StatusResult {
                version: env!("CARGO_PKG_VERSION").to_string(),
                proto_version: PROTO_VERSION,
                uptime_ms: ctx.started.elapsed().as_millis() as i64,
                event_count,
                db_path: ctx.db_path.display().to_string(),
            };
            Response::ok(req.id, serde_json::to_value(result).expect("serializes"))
        }
        methods::EVENTS_APPEND => {
            let ev: NewEvent = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad event: {e}"))
                }
            };
            if ev.kind.is_empty() || ev.source.is_empty() {
                return Response::err(req.id, errcodes::INVALID_REQUEST, "kind and source are required");
            }
            match ctx.ingest.ingest(ev).await {
                Ok(Some(event)) => {
                    ctx.mode.note_activity(event.ts);
                    Response::ok(req.id, serde_json::to_value(event).expect("serializes"))
                }
                // deliberately dropped (e.g. shell-hook loop guard)
                Ok(None) => Response::ok(req.id, serde_json::Value::Null),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::EVENTS_RECENT => {
            let params: RecentParams = if req.params.is_null() {
                RecentParams::default()
            } else {
                match serde_json::from_value(req.params) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::err(
                            req.id,
                            errcodes::INVALID_REQUEST,
                            format!("bad params: {e}"),
                        )
                    }
                }
            };
            match ctx.store.recent(params.limit.min(1000)).await {
                Ok(events) => Response::ok(req.id, serde_json::to_value(events).expect("serializes")),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::OBSERVATIONS_RECENT => {
            let params: ObsRecentParams = if req.params.is_null() {
                ObsRecentParams::default()
            } else {
                match serde_json::from_value(req.params) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::err(
                            req.id,
                            errcodes::INVALID_REQUEST,
                            format!("bad params: {e}"),
                        )
                    }
                }
            };
            match ctx.store.recent_observations(params.limit.min(1000), params.kind).await {
                Ok(obs) => Response::ok(req.id, serde_json::to_value(obs).expect("serializes")),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::PROJECTS_LIST => match ctx.store.list_projects().await {
            Ok(projects) => Response::ok(req.id, serde_json::to_value(projects).expect("serializes")),
            Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
        },
        methods::SESSIONS_RECENT => {
            let params: RecentParams = if req.params.is_null() {
                RecentParams::default()
            } else {
                match serde_json::from_value(req.params) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::err(
                            req.id,
                            errcodes::INVALID_REQUEST,
                            format!("bad params: {e}"),
                        )
                    }
                }
            };
            match ctx.store.recent_sessions(params.limit.min(1000)).await {
                Ok(sessions) => Response::ok(req.id, serde_json::to_value(sessions).expect("serializes")),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::MODE_GET => {
            Response::ok(req.id, serde_json::to_value(ctx.mode.state()).expect("serializes"))
        }
        other => {
            Response::err(req.id, errcodes::METHOD_NOT_FOUND, format!("unknown method: {other}"))
        }
    }
}
