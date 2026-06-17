use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rat_policy::{requires_slug, risk_tier, ActionKind, RiskOutcome};
use rat_proto::{
    errcodes, methods, AgentRunDto, ApprovalDto, ApprovalsDecideParams, DisclosureDto,
    DotfileEditDto, DotfileEditsApplyParams, DotfileEditsRevertParams, HelloParams, HelloResult,
    HitDto, LlmKeyPresence, LlmStatusResult, MemoryDto, MemoryListParams, MemorySearchParams,
    NewEvent, ObsRecentParams, PinDto, PinsPinRecentParams, PinsUnpinParams, PushbackDto,
    PushbackFeedbackParams, PushbacksRecentParams, RecentParams, Request, Response,
    RetentionStatusDto, StatusResult, TerminalDto, TerminalsSetRoleParams, VoiceBackendStatusDto,
    VoiceStatusDto, VoiceUtteranceDto, WorkbenchMergeBackParams, WorkbenchRunsParams,
    WorkbenchStartParams, WorkbenchTailParams, PROTO_VERSION,
};
use rat_workbench::runner::{ExecutionBackend, TaskRunner};
use rat_workbench::{ClaudeCode, Codex, FakeAgent};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use rat_core::clock::Clock;
use rat_memory::embed::EmbeddingClient;
use rat_memory::retrieve::{search, SearchParams};
use rat_memory::HitKind;
use rat_store::rows::{MemoryFilter, NewDotfileEdit};
use rat_store::store::Store;

use crate::ingest::Ingest;
use crate::mode::ModeManager;
use crate::pins::{media_from_str, PinKind, PinService};
use crate::sensors_health::SensorGate;

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
    pub sensors: Arc<SensorGate>,
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
            return Response::err(
                0,
                errcodes::INVALID_REQUEST,
                format!("invalid request: {e}"),
            )
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
                sensors: ctx.sensors.snapshot(),
            };
            Response::ok(req.id, serde_json::to_value(result).expect("serializes"))
        }
        methods::EVENTS_APPEND => {
            let ev: NewEvent = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad event: {e}"),
                    )
                }
            };
            if ev.kind.is_empty() || ev.source.is_empty() {
                return Response::err(
                    req.id,
                    errcodes::INVALID_REQUEST,
                    "kind and source are required",
                );
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
                Ok(events) => {
                    Response::ok(req.id, serde_json::to_value(events).expect("serializes"))
                }
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
            match ctx
                .store
                .recent_observations(params.limit.min(1000), params.kind)
                .await
            {
                Ok(obs) => Response::ok(req.id, serde_json::to_value(obs).expect("serializes")),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::PROJECTS_LIST => match ctx.store.list_projects().await {
            Ok(projects) => {
                Response::ok(req.id, serde_json::to_value(projects).expect("serializes"))
            }
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
                Ok(sessions) => {
                    Response::ok(req.id, serde_json::to_value(sessions).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::MODE_GET => Response::ok(
            req.id,
            serde_json::to_value(ctx.mode.state()).expect("serializes"),
        ),
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
                SearchParams {
                    query: params.query,
                    project_id: params.project_id,
                    n,
                },
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
        methods::MEMORY_LIST => {
            let params: MemoryListParams = if req.params.is_null() {
                MemoryListParams {
                    r#type: None,
                    project_id: None,
                    include_archived: false,
                    limit: None,
                }
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
            let limit = params.limit.unwrap_or(50).min(1000) as usize;
            match ctx
                .store
                .list_memories(MemoryFilter {
                    r#type: params.r#type,
                    project_id: params.project_id,
                    include_archived: params.include_archived,
                })
                .await
            {
                Ok(rows) => {
                    let dtos: Vec<MemoryDto> =
                        rows.into_iter().take(limit).map(memory_to_dto).collect();
                    Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::DISCLOSURES_RECENT => {
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
            match ctx.store.recent_disclosures(params.limit.min(1000)).await {
                Ok(rows) => {
                    let dtos: Vec<DisclosureDto> =
                        rows.into_iter().map(disclosure_to_dto).collect();
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
                embedding_enabled: ctx
                    .llm_status
                    .embedding_enabled
                    .load(std::sync::atomic::Ordering::Relaxed),
                critic_enabled: ctx.llm_status.critic_enabled,
                last_error,
            };
            Response::ok(req.id, serde_json::to_value(result).expect("serializes"))
        }
        methods::WORKBENCH_START => {
            let params: WorkbenchStartParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            // Resolve project_id → root_path
            let project = match ctx.store.get_project_by_id(params.project_id.clone()).await {
                Ok(Some(p)) => p,
                Ok(None) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("project {} not found", params.project_id),
                    )
                }
                Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            };
            let project_root = std::path::PathBuf::from(&project.root_path);
            let backend = match params.executor.as_str() {
                "local" => ExecutionBackend::Local,
                "docker" => {
                    let Some(image) = params
                        .docker_image
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    else {
                        return Response::err(
                            req.id,
                            errcodes::INVALID_REQUEST,
                            "docker executor requires docker_image",
                        );
                    };
                    ExecutionBackend::Docker {
                        image: image.to_string(),
                    }
                }
                other => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("unknown executor: {other}; must be local|docker"),
                    );
                }
            };
            let adapter: Box<dyn rat_workbench::AgentAdapter> = match params.adapter.as_str() {
                "fakeagent" => Box::new(FakeAgent::from_manifest()),
                "claude-code" => Box::new(ClaudeCode),
                "codex" => Box::new(Codex),
                other => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("unknown adapter: {other}; must be fakeagent|claude-code|codex"),
                    )
                }
            };
            let run = ctx
                .task_runner
                .start_with_backend(
                    &project_root,
                    &params.project_id,
                    &params.title,
                    adapter.as_ref(),
                    "HEAD",
                    backend,
                )
                .await;
            match run {
                Ok(r) => Response::ok(
                    req.id,
                    serde_json::to_value(agent_run_to_dto(r)).expect("serializes"),
                ),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::WORKBENCH_RUNS => {
            let params: WorkbenchRunsParams = if req.params.is_null() {
                WorkbenchRunsParams { n: None }
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
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            let run = match ctx.store.get_agent_run(params.run_id.clone()).await {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("run {} not found", params.run_id),
                    )
                }
                Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            };
            let target = match run.tmux_target {
                Some(t) => t,
                None => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        "run has no tmux_target",
                    )
                }
            };
            let lines = params.lines.unwrap_or(50);
            match ctx.task_runner.tmux.capture_tail(&target, lines) {
                Ok(output) => Response::ok(req.id, serde_json::json!({ "lines": output })),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::APPROVALS_PENDING => match ctx.store.pending_approvals().await {
            Ok(approvals) => {
                let dtos: Vec<ApprovalDto> = approvals.into_iter().map(approval_to_dto).collect();
                Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
            }
            Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
        },
        methods::APPROVALS_DECIDE => {
            let params: ApprovalsDecideParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            if params.verdict != "approve" && params.verdict != "deny" {
                return Response::err(
                    req.id,
                    errcodes::INVALID_REQUEST,
                    "verdict must be 'approve' or 'deny'",
                );
            }
            // Fetch the approval to check its tier and slug
            let approval = match ctx.store.get_approval(params.id.clone()).await {
                Ok(Some(a)) => a,
                Ok(None) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("approval {} not found", params.id),
                    )
                }
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
                let expected_slug: String = approval
                    .id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                match &params.slug {
                    None => {
                        return Response::err(
                            req.id,
                            errcodes::INVALID_REQUEST,
                            "R3 approval requires a slug confirmation",
                        )
                    }
                    Some(s) if s != &expected_slug => {
                        return Response::err(
                            req.id,
                            errcodes::INVALID_REQUEST,
                            format!("slug mismatch: expected '{expected_slug}'"),
                        )
                    }
                    Some(_) => {}
                }
            }
            let now = ctx.clock.now_ms();
            if params.verdict == "approve" {
                // Decide the approval as approved
                let decided = match ctx
                    .store
                    .decide_approval(
                        params.id.clone(),
                        "approved".to_string(),
                        now,
                        "cli".to_string(),
                        params.note.clone(),
                    )
                    .await
                {
                    Ok(a) => a,
                    Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
                };
                // If it's a merge_back approval, trigger execute_merge
                if decided.kind == "merge_back" {
                    match ctx.task_runner.execute_merge(&decided).await {
                        Ok(_outcome) => {}
                        Err(e) => {
                            return Response::err(
                                req.id,
                                errcodes::INTERNAL,
                                format!("execute_merge failed: {e}"),
                            )
                        }
                    }
                }
                // Re-fetch to return updated state
                match ctx.store.get_approval(params.id).await {
                    Ok(Some(a)) => Response::ok(
                        req.id,
                        serde_json::to_value(approval_to_dto(a)).expect("serializes"),
                    ),
                    _ => Response::ok(
                        req.id,
                        serde_json::to_value(approval_to_dto(decided)).expect("serializes"),
                    ),
                }
            } else {
                // deny
                match ctx
                    .task_runner
                    .deny(&params.id, params.note.as_deref())
                    .await
                {
                    Ok(a) => Response::ok(
                        req.id,
                        serde_json::to_value(approval_to_dto(a)).expect("serializes"),
                    ),
                    Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
                }
            }
        }
        methods::WORKBENCH_MERGE_BACK => {
            let params: WorkbenchMergeBackParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            match ctx.task_runner.merge_back(&params.run_id).await {
                Ok(approval) => Response::ok(
                    req.id,
                    serde_json::to_value(approval_to_dto(approval)).expect("serializes"),
                ),
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
                PinsPinRecentParams {
                    media: "screen".into(),
                    minutes: 5,
                }
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
            let media = match media_from_str(&params.media) {
                Ok(m) => m,
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, e.to_string()),
            };
            match service
                .pin_recent(media, params.minutes, PinKind::Manual, "manual pin_recent")
                .await
            {
                Ok(pin) => Response::ok(
                    req.id,
                    serde_json::to_value(pin_to_dto(pin)).expect("serializes"),
                ),
                Err(e) => Response::err(req.id, errcodes::INVALID_REQUEST, e.to_string()),
            }
        }
        methods::PINS_UNPIN => {
            let Some(service) = &ctx.pins else {
                return Response::err(req.id, errcodes::INTERNAL, "pin service unavailable");
            };
            let params: PinsUnpinParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            match service.unpin(&params.id).await {
                Ok(()) => Response::ok(req.id, serde_json::Value::Null),
                Err(e) => Response::err(req.id, errcodes::INVALID_REQUEST, e.to_string()),
            }
        }
        methods::RING_STATUS => {
            let Some(service) = &ctx.pins else {
                return Response::err(req.id, errcodes::INTERNAL, "pin service unavailable");
            };
            match service.ring_status() {
                Ok(status) => {
                    Response::ok(req.id, serde_json::to_value(status).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::RETENTION_STATUS => match ctx.store.get_retention_status().await {
            Ok(Some(status)) => {
                let dto = RetentionStatusDto {
                    last_run_ms: status.last_run_ms,
                    observations_deleted: status.observations_deleted,
                    pins_expired: status.pins_expired,
                    api_calls_deleted: status.api_calls_deleted,
                };
                Response::ok(req.id, serde_json::to_value(Some(dto)).expect("serializes"))
            }
            Ok(None) => Response::ok(req.id, serde_json::Value::Null),
            Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
        },
        methods::VOICE_STATUS => {
            let result = VoiceStatusDto {
                enabled: false,
                prewake_ring_secs: 8,
                backends: vec![
                    unavailable_backend("mic", "mic feature not built"),
                    unavailable_backend("wake", "wake feature not built"),
                    unavailable_backend("vad", "wake feature not built"),
                    unavailable_backend("stt", "stt feature not built"),
                    unavailable_backend("tts", "tts feature not built"),
                ],
            };
            Response::ok(req.id, serde_json::to_value(result).expect("serializes"))
        }
        methods::VOICE_UTTERANCES => {
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
            match ctx
                .store
                .recent_voice_utterances(params.limit.min(1000))
                .await
            {
                Ok(rows) => {
                    let dtos: Vec<VoiceUtteranceDto> =
                        rows.into_iter().map(voice_utterance_to_dto).collect();
                    Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::TERMINALS_LIST => match ctx.store.list_terminals().await {
            Ok(rows) => {
                let dtos: Vec<TerminalDto> = rows.into_iter().map(terminal_to_dto).collect();
                Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
            }
            Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
        },
        methods::TERMINALS_SET_ROLE => {
            let params: TerminalsSetRoleParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            if !matches!(
                params.role.as_str(),
                "operator" | "workbench" | "foreign" | "ignored"
            ) {
                return Response::err(
                    req.id,
                    errcodes::INVALID_REQUEST,
                    "role must be operator|workbench|foreign|ignored",
                );
            }
            let terminal = match ctx.store.get_terminal(params.id.clone()).await {
                Ok(Some(row)) => row,
                Ok(None) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("terminal {} not found", params.id),
                    )
                }
                Err(e) => return Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            };
            let updated = rat_store::rows::NewTerminal {
                tty: terminal.tty,
                pid: terminal.pid,
                emulator: terminal.emulator,
                tmux_target: terminal.tmux_target,
                role: params.role,
                project_id: terminal.project_id,
                cmd_hash: terminal.cmd_hash,
                meta: terminal.meta,
            };
            match ctx.store.upsert_terminal(updated).await {
                Ok(row) => Response::ok(
                    req.id,
                    serde_json::to_value(terminal_to_dto(row)).expect("serializes"),
                ),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::DOTFILE_EDITS_LIST => {
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
            match ctx.store.recent_dotfile_edits(params.limit.min(1000)).await {
                Ok(rows) => {
                    let dtos: Vec<DotfileEditDto> =
                        rows.into_iter().map(dotfile_edit_to_dto).collect();
                    Response::ok(req.id, serde_json::to_value(dtos).expect("serializes"))
                }
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::DOTFILE_EDITS_APPLY => {
            let params: DotfileEditsApplyParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            match apply_dotfile_edit(ctx, params).await {
                Ok(row) => Response::ok(
                    req.id,
                    serde_json::to_value(dotfile_edit_to_dto(row)).expect("serializes"),
                ),
                Err(e) => Response::err(req.id, errcodes::INVALID_REQUEST, e.to_string()),
            }
        }
        methods::DOTFILE_EDITS_REVERT => {
            let params: DotfileEditsRevertParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response::err(
                        req.id,
                        errcodes::INVALID_REQUEST,
                        format!("bad params: {e}"),
                    )
                }
            };
            match revert_dotfile_edit(ctx, &params.id).await {
                Ok(row) => Response::ok(
                    req.id,
                    serde_json::to_value(dotfile_edit_to_dto(row)).expect("serializes"),
                ),
                Err(e) => Response::err(req.id, errcodes::INVALID_REQUEST, e.to_string()),
            }
        }
        other => Response::err(
            req.id,
            errcodes::METHOD_NOT_FOUND,
            format!("unknown method: {other}"),
        ),
    }
}

async fn apply_dotfile_edit(
    ctx: &ServerCtx,
    params: DotfileEditsApplyParams,
) -> anyhow::Result<rat_store::rows::DotfileEdit> {
    let kind = config_kind_from_str(&params.kind)?;
    let snapshot = rat_dotfile::apply_atomic(
        std::path::Path::new(&params.path),
        kind,
        params.contents.as_bytes(),
        &rat_dotfile::PathCommandResolver,
    )?;
    let now = ctx.clock.now_ms();
    let before = ctx.store.insert_blob(snapshot.before, now).await?;
    let after = ctx.store.insert_blob(snapshot.after, now).await?;
    let row = ctx
        .store
        .insert_dotfile_edit(NewDotfileEdit {
            path: snapshot.path.display().to_string(),
            kind: params.kind,
            before_blob: before.id,
            after_blob: after.id,
            diff: snapshot.diff,
            reason: params.reason,
            source: params.source,
            risk: params.risk,
            applied: true,
            meta: params.meta,
        })
        .await?;
    Ok(row)
}

fn config_kind_from_str(kind: &str) -> anyhow::Result<rat_dotfile::ConfigKind> {
    match kind {
        "json" => Ok(rat_dotfile::ConfigKind::Json),
        "jsonc" => Ok(rat_dotfile::ConfigKind::Jsonc),
        "toml" => Ok(rat_dotfile::ConfigKind::Toml),
        "yaml" | "yml" => Ok(rat_dotfile::ConfigKind::Yaml),
        "text" => Ok(rat_dotfile::ConfigKind::Text),
        other => anyhow::bail!("unsupported config kind: {other}"),
    }
}

async fn revert_dotfile_edit(
    ctx: &ServerCtx,
    id: &str,
) -> anyhow::Result<rat_store::rows::DotfileEdit> {
    let original = ctx
        .store
        .get_dotfile_edit(id.to_string())
        .await?
        .ok_or_else(|| anyhow::anyhow!("dotfile edit {id} not found"))?;
    if original.reverted_by.is_some() {
        anyhow::bail!("dotfile edit {id} is already reverted");
    }
    let before_blob = ctx
        .store
        .get_blob(original.before_blob.clone())
        .await?
        .ok_or_else(|| anyhow::anyhow!("before blob {} not found", original.before_blob))?;
    let after_blob = ctx
        .store
        .get_blob(original.after_blob.clone())
        .await?
        .ok_or_else(|| anyhow::anyhow!("after blob {} not found", original.after_blob))?;

    let snapshot = rat_dotfile::DotfileSnapshot {
        path: std::path::PathBuf::from(&original.path),
        before: before_blob.bytes,
        after: after_blob.bytes,
        diff: original.diff.clone(),
    };
    let reverted = rat_dotfile::revert(&snapshot)?;
    let now = ctx.clock.now_ms();
    let current_blob = ctx.store.insert_blob(reverted.before.clone(), now).await?;
    let restored_blob = ctx.store.insert_blob(reverted.after.clone(), now).await?;
    let revert_row = ctx
        .store
        .insert_dotfile_edit(NewDotfileEdit {
            path: original.path.clone(),
            kind: original.kind.clone(),
            before_blob: current_blob.id,
            after_blob: restored_blob.id,
            diff: reverted.diff,
            reason: format!("revert {}", original.id),
            source: "rat-dotfile".to_string(),
            risk: original.risk,
            applied: true,
            meta: serde_json::json!({ "reverts": original.id }),
        })
        .await?;
    ctx.store
        .mark_dotfile_edit_reverted(original.id, revert_row.id.clone())
        .await?;
    Ok(revert_row)
}

fn unavailable_backend(name: &str, reason: &str) -> VoiceBackendStatusDto {
    VoiceBackendStatusDto {
        name: name.to_string(),
        state: "unavailable".to_string(),
        reason: Some(reason.to_string()),
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

fn memory_to_dto(memory: rat_store::rows::Memory) -> MemoryDto {
    MemoryDto {
        id: memory.id,
        r#type: memory.r#type,
        project_id: memory.project_id,
        title: memory.title,
        body: memory.body,
        confidence: memory.confidence,
        created: memory.created,
        updated: memory.updated,
        source_event_ids: memory.source_event_ids,
        archived: memory.archived,
    }
}

fn disclosure_to_dto(disclosure: rat_store::rows::Disclosure) -> DisclosureDto {
    DisclosureDto {
        id: disclosure.id,
        ts: disclosure.ts,
        api_call_id: disclosure.api_call_id,
        model: disclosure.model,
        purpose: disclosure.purpose,
        memory_ids: disclosure.memory_ids,
        observation_ids: disclosure.observation_ids,
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
    let spoken_slug = rat_voice::spoken_slug(&a.id);
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
        spoken_slug,
    }
}

fn voice_utterance_to_dto(v: rat_store::rows::VoiceUtterance) -> VoiceUtteranceDto {
    VoiceUtteranceDto {
        id: v.id,
        ts: v.ts,
        lang: v.lang,
        text: v.text,
        intent: v.intent,
        wake_word: v.wake_word,
        handled: v.handled,
    }
}

fn terminal_to_dto(t: rat_store::rows::Terminal) -> TerminalDto {
    TerminalDto {
        id: t.id,
        tty: t.tty,
        pid: t.pid,
        emulator: t.emulator,
        tmux_target: t.tmux_target,
        role: t.role,
        project_id: t.project_id,
        cmd_hash: t.cmd_hash,
        first_seen: t.first_seen,
        last_seen: t.last_seen,
        meta: t.meta,
    }
}

fn dotfile_edit_to_dto(e: rat_store::rows::DotfileEdit) -> DotfileEditDto {
    DotfileEditDto {
        id: e.id,
        path: e.path,
        kind: e.kind,
        before_blob: e.before_blob,
        after_blob: e.after_blob,
        diff: e.diff,
        reason: e.reason,
        source: e.source,
        risk: e.risk,
        created: e.created,
        applied: e.applied,
        reverted_by: e.reverted_by,
        meta: e.meta,
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
