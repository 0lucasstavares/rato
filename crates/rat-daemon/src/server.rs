use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rat_proto::{
    errcodes, methods, HelloParams, HelloResult, HitDto, LlmKeyPresence, LlmStatusResult,
    MemorySearchParams, NewEvent, ObsRecentParams, PushbackDto, PushbackFeedbackParams,
    PushbacksRecentParams, RecentParams, Request, Response, StatusResult, PROTO_VERSION,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use rat_core::clock::Clock;
use rat_memory::embed::EmbeddingClient;
use rat_memory::retrieve::{search, SearchParams};
use rat_memory::HitKind;
use rat_store::store::Store;

use crate::ingest::Ingest;
use crate::mode::ModeManager;

/// Shared LLM/critic status, readable via the `llm.status` RPC method.
pub struct LlmStatusState {
    pub provider: String,
    /// Atomic: flipped to false at runtime when the account rejects the
    /// embedding model (4xx) — retrieval then runs FTS-only per spec.
    pub embedding_enabled: std::sync::atomic::AtomicBool,
    pub critic_enabled: bool,
    pub last_error: Mutex<Option<String>>,
    pub openai_key: bool,
    pub anthropic_key: bool,
    pub openrouter_key: bool,
}

impl LlmStatusState {
    /// Convenience constructor for the disabled/no-LLM case.
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self {
            provider: "openai".to_string(),
            embedding_enabled: std::sync::atomic::AtomicBool::new(false),
            critic_enabled: false,
            last_error: Mutex::new(None),
            openai_key: false,
            anthropic_key: false,
            openrouter_key: false,
        })
    }
}

pub struct ServerCtx {
    pub store: Store,
    pub ingest: Arc<Ingest>,
    pub mode: Arc<ModeManager>,
    pub started: Instant,
    pub db_path: PathBuf,
    pub clock: Arc<dyn Clock>,
    pub embedder: Option<EmbeddingClient>,
    pub llm_status: Arc<LlmStatusState>,
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
        methods::MEMORY_SEARCH => {
            let params: MemorySearchParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            let n = params.n.unwrap_or(8) as usize;
            match search(
                &ctx.store,
                ctx.embedder.as_ref(),
                &ctx.clock,
                SearchParams { query: params.query, project_id: params.project_id, n },
            )
            .await
            {
                Ok(hits) => {
                    let dtos: Vec<HitDto> = hits
                        .into_iter()
                        .map(|h| HitDto {
                            id: h.id,
                            kind: match h.kind {
                                HitKind::Observation => "observation".to_string(),
                                HitKind::Memory => "memory".to_string(),
                            },
                            score: h.score,
                        })
                        .collect();
                    Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::PUSHBACKS_RECENT => {
            let params: PushbacksRecentParams = if req.params.is_null() {
                PushbacksRecentParams { n: None }
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
            let limit = params.n.unwrap_or(10);
            match ctx.store.recent_pushbacks(limit).await {
                Ok(pbs) => {
                    let dtos: Vec<PushbackDto> = pbs.into_iter().map(pushback_to_dto).collect();
                    Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::PUSHBACKS_FEEDBACK => {
            let params: PushbackFeedbackParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            let status = match params.verdict.as_str() {
                "useful" => "accepted",
                "dismiss" => "dismissed",
                "snooze" => "snoozed",
                _ => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        "verdict must be useful|dismiss|snooze",
                    )
                }
            };
            let pb = match ctx.store.get_pushback(params.id.clone()).await {
                Ok(Some(pb)) => pb,
                Ok(None) => {
                    return Response::err(req.id, errcodes::INVALID_REQUEST, "pushback not found")
                }
                Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            };
            let now = ctx.clock.now_ms();
            let latency_ms = now - pb.ts;
            match ctx
                .store
                .pushback_feedback(params.id.clone(), status.to_string(), now, latency_ms)
                .await
            {
                Ok(()) => match ctx.store.get_pushback(params.id).await {
                    Ok(Some(updated)) => Response::ok(
                        req.id,
                        serde_json::to_value(pushback_to_dto(updated)).expect("serializes"),
                    ),
                    _ => Response::err(req.id, errcodes::INTERNAL, "failed to re-fetch"),
                },
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::LLM_STATUS => {
            let last_error = ctx.llm_status.last_error.lock().unwrap().clone();
            let result = LlmStatusResult {
                provider: ctx.llm_status.provider.clone(),
                keys: LlmKeyPresence {
                    openai: ctx.llm_status.openai_key,
                    anthropic: ctx.llm_status.anthropic_key,
                    openrouter: ctx.llm_status.openrouter_key,
                },
                embedding_enabled: ctx.llm_status.embedding_enabled.load(std::sync::atomic::Ordering::Relaxed),
                critic_enabled: ctx.llm_status.critic_enabled,
                last_error,
            };
            Response::ok(req.id, serde_json::to_value(result).expect("serializes"))
        }
        other => {
            Response::err(req.id, errcodes::METHOD_NOT_FOUND, format!("unknown method: {other}"))
        }
    }
}

fn pushback_to_dto(pb: rat_store::rows::Pushback) -> PushbackDto {
    PushbackDto {
        id: pb.id,
        ts: pb.ts,
        mode: pb.mode,
        trigger: pb.trigger,
        severity: pb.severity,
        title: pb.title,
        message_en: pb.message_en,
        message_pt: pb.message_pt,
        evidence: pb.evidence,
        proposals: pb.proposals,
        confidence: pb.confidence,
        status: pb.status,
        decided_at: pb.decided_at,
        latency_ms: pb.latency_ms,
    }
}
