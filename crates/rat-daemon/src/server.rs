use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rat_proto::{
    AgentRunDto, ApprovalDto, ApprovalsDecideParams, WorkbenchMergeBackParams, WorkbenchRunsParams,
    WorkbenchStartParams, WorkbenchTailParams, errcodes, methods, HelloParams, HelloResult, HitDto,
    LlmKeyPresence, LlmStatusResult, MemorySearchParams, NewEvent, ObsRecentParams, PushbackDto,
    PushbackFeedbackParams, PushbacksRecentParams, RecentParams, Request, Response, StatusResult,
    PinDto, PinsPinRecentParams, PinsUnpinParams, PROTO_VERSION,
};
use rat_policy::{requires_slug, risk_tier, ActionKind, RiskOutcome};
use rat_workbench::runner::TaskRunner;
use rat_workbench::{ClaudeCode, Codex, FakeAgent};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use rat_core::clock::Clock;
use rat_memory::embed::EmbeddingClient;
use rat_memory::retrieve::{search, SearchParams};
use rat_memory::HitKind;
use rat_store::store::Store;

use crate::ingest::Ingest;
use crate::mode::ModeManager;
use crate::pins::{media_from_str, PinKind, PinService};

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
    pub task_runner: TaskRunner,
    pub pins: Option<PinService>,
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
        methods::WORKBENCH_START => {
            let params: WorkbenchStartParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad params: {e}")),
            };
            // Resolve project_id → root_path
            let project = match ctx.store.get_project_by_id(params.project_id.clone()).await {
                Ok(Some(p)) => p,
                Ok(None) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("project {} not found", params.project_id)),
                Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            };
            let project_root = std::path::PathBuf::from(&project.root_path);
            // Map adapter string to impl
            let run = match params.adapter.as_str() {
                "fakeagent" => {
                    let adapter = FakeAgent::from_manifest();
                    ctx.task_runner.start(&project_root, &params.project_id, &params.title, &adapter, "HEAD").await
                }
                "claude-code" => {
                    let adapter = ClaudeCode;
                    ctx.task_runner.start(&project_root, &params.project_id, &params.title, &adapter, "HEAD").await
                }
                "codex" => {
                    let adapter = Codex;
                    ctx.task_runner.start(&project_root, &params.project_id, &params.title, &adapter, "HEAD").await
                }
                other => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("unknown adapter: {other}; must be fakeagent|claude-code|codex")),
            };
            match run {
                Ok(r) => Response::ok(req.id, serde_json::to_value(agent_run_to_dto(r)).expect("serializes")),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::WORKBENCH_RUNS => {
            let params: WorkbenchRunsParams = if req.params.is_null() {
                WorkbenchRunsParams { n: None }
            } else {
                match serde_json::from_value(req.params) {
                    Ok(p) => p,
                    Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad params: {e}")),
                }
            };
            let n = params.n.unwrap_or(20);
            // Poll-on-read: advance any running runs before listing so the
            // caller sees immediately-accurate status without waiting for the
            // 3s background sweep.
            if let Ok(running) = ctx.store.recent_agent_runs(100).await {
                for run in running.into_iter().filter(|r| r.status == "running") {
                    if let Err(e) = ctx.task_runner.poll(&run.id).await {
                        tracing::debug!("workbench.runs poll({}): {e}", run.id);
                    }
                }
            }
            match ctx.store.recent_agent_runs(n).await {
                Ok(runs) => {
                    let dtos: Vec<AgentRunDto> = runs.into_iter().map(agent_run_to_dto).collect();
                    Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::WORKBENCH_TAIL => {
            let params: WorkbenchTailParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad params: {e}")),
            };
            let run = match ctx.store.get_agent_run(params.run_id.clone()).await {
                Ok(Some(r)) => r,
                Ok(None) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("run {} not found", params.run_id)),
                Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            };
            let target = match run.tmux_target {
                Some(t) => t,
                None => return Response::err(req.id, errcodes::INVALID_REQUEST, "run has no tmux_target"),
            };
            let lines = params.lines.unwrap_or(50);
            match ctx.task_runner.tmux.capture_tail(&target, lines) {
                Ok(output) => Response::ok(req.id, serde_json::json!({ "lines": output })),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::APPROVALS_PENDING => {
            match ctx.store.pending_approvals().await {
                Ok(approvals) => {
                    let dtos: Vec<ApprovalDto> = approvals.into_iter().map(approval_to_dto).collect();
                    Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::APPROVALS_DECIDE => {
            let params: ApprovalsDecideParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad params: {e}")),
            };
            if params.verdict != "approve" && params.verdict != "deny" {
                return Response::err(req.id, errcodes::INVALID_REQUEST, "verdict must be 'approve' or 'deny'");
            }
            // Fetch the approval to check its tier and slug
            let approval = match ctx.store.get_approval(params.id.clone()).await {
                Ok(Some(a)) => a,
                Ok(None) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("approval {} not found", params.id)),
                Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            };
            // R3 gate: if risk tier requires slug, slug param MUST be provided and match
            // The slug is the last 6 chars of the approval id
            let tier = match risk_tier(ActionKind::MergeBack) {
                RiskOutcome::Tier(t) => t,
                RiskOutcome::Refused => unreachable!(),
            };
            // Check the actual risk stored on the approval
            let approval_tier = match approval.risk {
                0 => rat_policy::Tier::R0,
                1 => rat_policy::Tier::R1,
                2 => rat_policy::Tier::R2,
                3 => rat_policy::Tier::R3,
                _ => rat_policy::Tier::R2,
            };
            let _ = tier; // suppress unused
            if requires_slug(approval_tier) {
                let expected_slug: String = approval.id.chars().rev().take(6).collect::<String>().chars().rev().collect();
                match &params.slug {
                    None => return Response::err(req.id, errcodes::INVALID_REQUEST, "R3 approval requires a slug confirmation"),
                    Some(s) if s != &expected_slug => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("slug mismatch: expected '{expected_slug}'")),
                    Some(_) => {}
                }
            }
            let now = ctx.clock.now_ms();
            if params.verdict == "approve" {
                // Decide the approval as approved
                let decided = match ctx.store.decide_approval(
                    params.id.clone(),
                    "approved".to_string(),
                    now,
                    "cli".to_string(),
                    params.note.clone(),
                ).await {
                    Ok(a) => a,
                    Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
                };
                // If it's a merge_back approval, trigger execute_merge
                if decided.kind == "merge_back" {
                    match ctx.task_runner.execute_merge(&decided).await {
                        Ok(_outcome) => {}
                        Err(e) => return Response::err(req.id, errcodes::INTERNAL, format!("execute_merge failed: {e}")),
                    }
                }
                // Re-fetch to return updated state
                match ctx.store.get_approval(params.id).await {
                    Ok(Some(a)) => Response::ok(req.id, serde_json::to_value(approval_to_dto(a)).expect("serializes")),
                    _ => Response::ok(req.id, serde_json::to_value(approval_to_dto(decided)).expect("serializes")),
                }
            } else {
                // deny
                match ctx.task_runner.deny(&params.id, params.note.as_deref()).await {
                    Ok(a) => Response::ok(req.id, serde_json::to_value(approval_to_dto(a)).expect("serializes")),
                    Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
                }
            }
        }
        methods::WORKBENCH_MERGE_BACK => {
            let params: WorkbenchMergeBackParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad params: {e}")),
            };
            match ctx.task_runner.merge_back(&params.run_id).await {
                Ok(approval) => Response::ok(req.id, serde_json::to_value(approval_to_dto(approval)).expect("serializes")),
                Err(e) => Response::err(req.id, errcodes::INVALID_REQUEST, e.to_string()),
            }
        }
        methods::PINS_LIST => {
            let Some(service) = &ctx.pins else {
                return Response::err(req.id, errcodes::INTERNAL, "pin service unavailable");
            };
            match service.list().await {
                Ok(pins) => {
                    let dtos: Vec<PinDto> = pins.into_iter().map(pin_to_dto).collect();
                    Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::PINS_PIN_RECENT => {
            let Some(service) = &ctx.pins else {
                return Response::err(req.id, errcodes::INTERNAL, "pin service unavailable");
            };
            let params: PinsPinRecentParams = if req.params.is_null() {
                PinsPinRecentParams { media: "screen".into(), minutes: 5 }
            } else {
                match serde_json::from_value(req.params) {
                    Ok(p) => p,
                    Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad params: {e}")),
                }
            };
            let media = match media_from_str(&params.media) {
                Ok(m) => m,
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, e.to_string()),
            };
            match service.pin_recent(media, params.minutes, PinKind::Manual, "manual pin_recent").await {
                Ok(pin) => Response::ok(req.id, serde_json::to_value(pin_to_dto(pin)).expect("serializes")),
                Err(e) => Response::err(req.id, errcodes::INVALID_REQUEST, e.to_string()),
            }
        }
        methods::PINS_UNPIN => {
            let Some(service) = &ctx.pins else {
                return Response::err(req.id, errcodes::INTERNAL, "pin service unavailable");
            };
            let params: PinsUnpinParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad params: {e}")),
            };
            match service.unpin(&params.id).await {
                Ok(()) => Response::ok(req.id, serde_json::Value::Null),
                Err(e) => Response::err(req.id, errcodes::INVALID_REQUEST, e.to_string()),
            }
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

fn agent_run_to_dto(r: rat_store::rows::AgentRun) -> AgentRunDto {
    AgentRunDto {
        id: r.id,
        adapter: r.adapter,
        task_title: r.task_title,
        project_id: r.project_id,
        worktree_path: r.worktree_path,
        branch: r.branch,
        tmux_target: r.tmux_target,
        mode: r.mode,
        status: r.status,
        tokens: r.tokens,
        cost_usd: r.cost_usd,
        started: r.started,
        ended: r.ended,
        result_summary: r.result_summary,
        diffstat: r.diffstat,
    }
}

fn approval_to_dto(a: rat_store::rows::Approval) -> ApprovalDto {
    ApprovalDto {
        id: a.id,
        created: a.created,
        kind: a.kind,
        risk: a.risk,
        title: a.title,
        reason: a.reason,
        cwd: a.cwd,
        target: a.target,
        agent_identity: a.agent_identity,
        payload: a.payload,
        expected_impact: a.expected_impact,
        expires_at: a.expires_at,
        status: a.status,
        decided_at: a.decided_at,
        decided_via: a.decided_via,
        decision_note: a.decision_note,
        execution: a.execution,
    }
}

fn pin_to_dto(p: rat_store::rows::Pin) -> PinDto {
    PinDto {
        id: p.id,
        kind: p.kind,
        media: p.media,
        path: p.path,
        created: p.created,
        expires_at: p.expires_at,
        reason: p.reason,
        meta: p.meta,
    }
}
