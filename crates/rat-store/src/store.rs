use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use rusqlite::{params, Connection};
use tokio::sync::oneshot;

use rat_core::clock::Clock;
use rat_core::id::new_id;
use rat_proto::{Event, NewEvent, NewObservation, Observation, Project, WorkSession};

use crate::db::open_db;
use crate::error::StoreError;
use crate::rows::{
    decode_embedding, encode_embedding, AgentRun, Approval, Blob, Memory, MemoryFilter, NewAgentRun,
    NewApproval, NewMemory, NewPushback, Pushback,
};

type Reply<T> = oneshot::Sender<Result<T, StoreError>>;

enum Cmd {
    Append { ev: NewEvent, reply: Reply<Event> },
    Recent { limit: u32, reply: Reply<Vec<Event>> },
    Count { reply: Reply<u64> },
    UpsertProject { root_path: String, name: String, reply: Reply<Project> },
    ListProjects { reply: Reply<Vec<Project>> },
    AddObservation { obs: NewObservation, reply: Reply<Observation> },
    RecentObservations { limit: u32, kind: Option<String>, reply: Reply<Vec<Observation>> },
    SessionOpen { ws: WorkSession, reply: Reply<()> },
    SessionTouch { id: String, last_activity: i64, commands: u32, reply: Reply<()> },
    SessionClose { id: String, ended: i64, reply: Reply<()> },
    RecentSessions { limit: u32, reply: Reply<Vec<WorkSession>> },
    OpenSessions { reply: Reply<Vec<WorkSession>> },
    // v3 commands
    AddMemory { mem: NewMemory, reply: Reply<Memory> },
    UpdateMemoryConfidence { id: String, confidence: f64, reply: Reply<()> },
    ArchiveMemory { id: String, reply: Reply<()> },
    ListMemories { filter: MemoryFilter, reply: Reply<Vec<Memory>> },
    FtsObservations { query: String, limit: u32, reply: Reply<Vec<String>> },
    FtsMemories { query: String, limit: u32, reply: Reply<Vec<String>> },
    UnembeddedObservations { kinds: Vec<String>, limit: u32, reply: Reply<Vec<Observation>> },
    SetObservationEmbedding { obs_id: String, embedding: Vec<f32>, reply: Reply<()> },
    SetMemoryEmbedding { memory_id: String, embedding: Vec<f32>, reply: Reply<()> },
    AllObservationEmbeddings { limit: u32, reply: Reply<Vec<(String, Vec<f32>)>> },
    AllMemoryEmbeddings { limit: u32, reply: Reply<Vec<(String, Vec<f32>)>> },
    ObservationsByIds { ids: Vec<String>, reply: Reply<Vec<Observation>> },
    InsertPushback { pb: NewPushback, reply: Reply<Pushback> },
    RecentPushbacks { limit: u32, reply: Reply<Vec<Pushback>> },
    GetPushback { id: String, reply: Reply<Option<Pushback>> },
    PushbackFeedback { id: String, status: String, decided_at: i64, latency_ms: i64, reply: Reply<()> },
    PushbacksSince { ts: i64, reply: Reply<Vec<Pushback>> },
    InsertApiCall {
        model: String,
        purpose: String,
        tokens_in: Option<i64>,
        tokens_out: Option<i64>,
        cost_usd: Option<f64>,
        ok: bool,
        error: Option<String>,
        reply: Reply<String>,
    },
    InsertDisclosure {
        api_call_id: Option<String>,
        model: String,
        purpose: String,
        memory_ids: Vec<String>,
        observation_ids: Vec<String>,
        reply: Reply<String>,
    },
    ClosedSessionsWithoutSummary { limit: u32, reply: Reply<Vec<WorkSession>> },
    SetSessionSummary { id: String, summary: String, reply: Reply<()> },
    ObservationsBetween {
        project_id: String,
        from_ms: i64,
        to_ms: i64,
        limit: u32,
        reply: Reply<Vec<Observation>>,
    },
    DeleteObservationsOlderThan {
        cutoff_ms: i64,
        protected_ids: Vec<String>,
        max_rows: u32,
        reply: Reply<u32>,
    },
    // v4 — approvals
    InsertApproval { new_approval: NewApproval, reply: Reply<Approval> },
    PendingApprovals { reply: Reply<Vec<Approval>> },
    GetApproval { id: String, reply: Reply<Option<Approval>> },
    DecideApproval {
        id: String,
        status: String,
        decided_at: i64,
        decided_via: String,
        note: Option<String>,
        reply: Reply<Approval>,
    },
    SetApprovalExecution { id: String, execution: serde_json::Value, reply: Reply<()> },
    ExpireApprovals { now_ms: i64, reply: Reply<u32> },
    // v4 — agent_runs
    InsertAgentRun { run: NewAgentRun, reply: Reply<AgentRun> },
    UpdateAgentRunStatus {
        id: String,
        status: String,
        ended: Option<i64>,
        result_summary: Option<String>,
        diffstat: Option<serde_json::Value>,
        reply: Reply<()>,
    },
    RecentAgentRuns { n: u32, reply: Reply<Vec<AgentRun>> },
    GetAgentRun { id: String, reply: Reply<Option<AgentRun>> },
    // v4 — blobs
    InsertBlob { bytes: Vec<u8>, created: i64, reply: Reply<Blob> },
    GetBlob { id: String, reply: Reply<Option<Blob>> },
}

/// Cloneable handle to the single-writer SQLite actor thread.
/// All DB access funnels through one std thread that owns the Connection.
#[derive(Clone)]
pub struct Store {
    tx: mpsc::Sender<Cmd>,
}

impl Store {
    pub fn open(path: &Path, clock: Arc<dyn Clock>) -> Result<Self, StoreError> {
        let conn = open_db(path)?;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("rat-store".into())
            .spawn(move || actor_loop(conn, clock, rx))
            .expect("failed to spawn store thread");
        Ok(Self { tx })
    }

    pub async fn append(&self, ev: NewEvent) -> Result<Event, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::Append { ev, reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn recent(&self, limit: u32) -> Result<Vec<Event>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::Recent { limit, reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn count(&self) -> Result<u64, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::Count { reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn upsert_project(&self, root_path: String, name: String) -> Result<Project, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::UpsertProject { root_path, name, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::ListProjects { reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn add_observation(&self, obs: NewObservation) -> Result<Observation, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::AddObservation { obs, reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn recent_observations(
        &self,
        limit: u32,
        kind: Option<String>,
    ) -> Result<Vec<Observation>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::RecentObservations { limit, kind, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn session_open(&self, ws: WorkSession) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::SessionOpen { ws, reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn session_touch(
        &self,
        id: String,
        last_activity: i64,
        commands: u32,
    ) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::SessionTouch { id, last_activity, commands, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn session_close(&self, id: String, ended: i64) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::SessionClose { id, ended, reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn recent_sessions(&self, limit: u32) -> Result<Vec<WorkSession>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::RecentSessions { limit, reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn open_sessions(&self) -> Result<Vec<WorkSession>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::OpenSessions { reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    // -----------------------------------------------------------------------
    // v3 store API
    // -----------------------------------------------------------------------

    pub async fn add_memory(&self, mem: NewMemory) -> Result<Memory, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send(Cmd::AddMemory { mem, reply: rtx }).map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn update_memory_confidence(
        &self,
        id: String,
        confidence: f64,
    ) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::UpdateMemoryConfidence { id, confidence, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn archive_memory(&self, id: String) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::ArchiveMemory { id, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn list_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::ListMemories { filter, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    /// Full-text search over observations.
    /// ids ordered best-match first; consumer derives 1-based rank from position.
    pub async fn fts_observations(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<String>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::FtsObservations { query, limit, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    /// Full-text search over memories.
    /// ids ordered best-match first; consumer derives 1-based rank from position.
    pub async fn fts_memories(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<String>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::FtsMemories { query, limit, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn unembedded_observations(
        &self,
        kinds: Vec<String>,
        limit: u32,
    ) -> Result<Vec<Observation>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::UnembeddedObservations { kinds, limit, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn set_observation_embedding(
        &self,
        obs_id: String,
        embedding: Vec<f32>,
    ) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::SetObservationEmbedding { obs_id, embedding, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn set_memory_embedding(
        &self,
        memory_id: String,
        embedding: Vec<f32>,
    ) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::SetMemoryEmbedding { memory_id, embedding, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn all_observation_embeddings(
        &self,
        limit: u32,
    ) -> Result<Vec<(String, Vec<f32>)>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::AllObservationEmbeddings { limit, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn all_memory_embeddings(
        &self,
        limit: u32,
    ) -> Result<Vec<(String, Vec<f32>)>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::AllMemoryEmbeddings { limit, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn observations_by_ids(
        &self,
        ids: Vec<String>,
    ) -> Result<Vec<Observation>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::ObservationsByIds { ids, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn insert_pushback(&self, pb: NewPushback) -> Result<Pushback, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::InsertPushback { pb, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn recent_pushbacks(&self, limit: u32) -> Result<Vec<Pushback>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::RecentPushbacks { limit, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn get_pushback(&self, id: String) -> Result<Option<Pushback>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::GetPushback { id, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn pushback_feedback(
        &self,
        id: String,
        status: String,
        decided_at: i64,
        latency_ms: i64,
    ) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::PushbackFeedback { id, status, decided_at, latency_ms, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn pushbacks_since(&self, ts: i64) -> Result<Vec<Pushback>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::PushbacksSince { ts, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_api_call(
        &self,
        model: String,
        purpose: String,
        tokens_in: Option<i64>,
        tokens_out: Option<i64>,
        cost_usd: Option<f64>,
        ok: bool,
        error: Option<String>,
    ) -> Result<String, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::InsertApiCall { model, purpose, tokens_in, tokens_out, cost_usd, ok, error, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn insert_disclosure(
        &self,
        api_call_id: Option<String>,
        model: String,
        purpose: String,
        memory_ids: Vec<String>,
        observation_ids: Vec<String>,
    ) -> Result<String, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::InsertDisclosure {
                api_call_id,
                model,
                purpose,
                memory_ids,
                observation_ids,
                reply: rtx,
            })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn closed_sessions_without_summary(
        &self,
        limit: u32,
    ) -> Result<Vec<WorkSession>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::ClosedSessionsWithoutSummary { limit, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn set_session_summary(
        &self,
        id: String,
        summary: String,
    ) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::SetSessionSummary { id, summary, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn observations_between(
        &self,
        project_id: &str,
        from_ms: i64,
        to_ms: i64,
        limit: u32,
    ) -> Result<Vec<Observation>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::ObservationsBetween {
                project_id: project_id.to_string(),
                from_ms,
                to_ms,
                limit,
                reply: rtx,
            })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn delete_observations_older_than(
        &self,
        cutoff_ms: i64,
        protected_ids: &[String],
        max_rows: u32,
    ) -> Result<u32, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::DeleteObservationsOlderThan {
                cutoff_ms,
                protected_ids: protected_ids.to_vec(),
                max_rows,
                reply: rtx,
            })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    // -----------------------------------------------------------------------
    // v4 store API — approvals
    // -----------------------------------------------------------------------

    pub async fn insert_approval(&self, new_approval: NewApproval) -> Result<Approval, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::InsertApproval { new_approval, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn pending_approvals(&self) -> Result<Vec<Approval>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::PendingApprovals { reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn get_approval(&self, id: String) -> Result<Option<Approval>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::GetApproval { id, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    /// Decide a pending approval. Returns error if the approval is not in `pending` status.
    pub async fn decide_approval(
        &self,
        id: String,
        status: String,
        decided_at: i64,
        decided_via: String,
        note: Option<String>,
    ) -> Result<Approval, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::DecideApproval { id, status, decided_at, decided_via, note, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn set_approval_execution(
        &self,
        id: String,
        execution: serde_json::Value,
    ) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::SetApprovalExecution { id, execution, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    /// Expire all `pending` approvals whose `expires_at <= now_ms`. Returns count expired.
    pub async fn expire_approvals(&self, now_ms: i64) -> Result<u32, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::ExpireApprovals { now_ms, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    // -----------------------------------------------------------------------
    // v4 store API — agent_runs
    // -----------------------------------------------------------------------

    pub async fn insert_agent_run(&self, run: NewAgentRun) -> Result<AgentRun, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::InsertAgentRun { run, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn update_agent_run_status(
        &self,
        id: String,
        status: String,
        ended: Option<i64>,
        result_summary: Option<String>,
        diffstat: Option<serde_json::Value>,
    ) -> Result<(), StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::UpdateAgentRunStatus { id, status, ended, result_summary, diffstat, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn recent_agent_runs(&self, n: u32) -> Result<Vec<AgentRun>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::RecentAgentRuns { n, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn get_agent_run(&self, id: String) -> Result<Option<AgentRun>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::GetAgentRun { id, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    // -----------------------------------------------------------------------
    // v4 store API — blobs
    // -----------------------------------------------------------------------

    /// Insert a blob, deduplicating on sha256.
    /// If a blob with the same content already exists, returns the existing row.
    pub async fn insert_blob(&self, bytes: Vec<u8>, created: i64) -> Result<Blob, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::InsertBlob { bytes, created, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    pub async fn get_blob(&self, id: String) -> Result<Option<Blob>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::GetBlob { id, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }
}

fn actor_loop(conn: Connection, clock: Arc<dyn Clock>, rx: mpsc::Receiver<Cmd>) {
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Append { ev, reply } => {
                let _ = reply.send(do_append(&conn, clock.as_ref(), ev));
            }
            Cmd::Recent { limit, reply } => {
                let _ = reply.send(do_recent(&conn, limit));
            }
            Cmd::Count { reply } => {
                let _ = reply.send(do_count(&conn));
            }
            Cmd::UpsertProject { root_path, name, reply } => {
                let _ = reply.send(do_upsert_project(&conn, clock.as_ref(), &root_path, &name));
            }
            Cmd::ListProjects { reply } => {
                let _ = reply.send(do_list_projects(&conn));
            }
            Cmd::AddObservation { obs, reply } => {
                let _ = reply.send(do_add_observation(&conn, clock.as_ref(), obs));
            }
            Cmd::RecentObservations { limit, kind, reply } => {
                let _ = reply.send(do_recent_observations(&conn, limit, kind.as_deref()));
            }
            Cmd::SessionOpen { ws, reply } => {
                let _ = reply.send(do_session_open(&conn, &ws));
            }
            Cmd::SessionTouch { id, last_activity, commands, reply } => {
                let _ = reply.send(do_session_touch(&conn, &id, last_activity, commands));
            }
            Cmd::SessionClose { id, ended, reply } => {
                let _ = reply.send(do_session_close(&conn, &id, ended));
            }
            Cmd::RecentSessions { limit, reply } => {
                let _ = reply.send(do_sessions(&conn, Some(limit)));
            }
            Cmd::OpenSessions { reply } => {
                let _ = reply.send(do_sessions(&conn, None));
            }
            // v3
            Cmd::AddMemory { mem, reply } => {
                let _ = reply.send(do_add_memory(&conn, clock.as_ref(), mem));
            }
            Cmd::UpdateMemoryConfidence { id, confidence, reply } => {
                let _ = reply.send(do_update_memory_confidence(&conn, clock.as_ref(), &id, confidence));
            }
            Cmd::ArchiveMemory { id, reply } => {
                let _ = reply.send(do_archive_memory(&conn, clock.as_ref(), &id));
            }
            Cmd::ListMemories { filter, reply } => {
                let _ = reply.send(do_list_memories(&conn, &filter));
            }
            Cmd::FtsObservations { query, limit, reply } => {
                let _ = reply.send(do_fts_observations(&conn, &query, limit));
            }
            Cmd::FtsMemories { query, limit, reply } => {
                let _ = reply.send(do_fts_memories(&conn, &query, limit));
            }
            Cmd::UnembeddedObservations { kinds, limit, reply } => {
                let _ = reply.send(do_unembedded_observations(&conn, &kinds, limit));
            }
            Cmd::SetObservationEmbedding { obs_id, embedding, reply } => {
                let _ = reply.send(do_set_observation_embedding(&conn, &obs_id, &embedding));
            }
            Cmd::SetMemoryEmbedding { memory_id, embedding, reply } => {
                let _ = reply.send(do_set_memory_embedding(&conn, &memory_id, &embedding));
            }
            Cmd::AllObservationEmbeddings { limit, reply } => {
                let _ = reply.send(do_all_observation_embeddings(&conn, limit));
            }
            Cmd::AllMemoryEmbeddings { limit, reply } => {
                let _ = reply.send(do_all_memory_embeddings(&conn, limit));
            }
            Cmd::ObservationsByIds { ids, reply } => {
                let _ = reply.send(do_observations_by_ids(&conn, &ids));
            }
            Cmd::InsertPushback { pb, reply } => {
                let _ = reply.send(do_insert_pushback(&conn, clock.as_ref(), pb));
            }
            Cmd::RecentPushbacks { limit, reply } => {
                let _ = reply.send(do_recent_pushbacks(&conn, limit));
            }
            Cmd::GetPushback { id, reply } => {
                let _ = reply.send(do_get_pushback(&conn, &id));
            }
            Cmd::PushbackFeedback { id, status, decided_at, latency_ms, reply } => {
                let _ = reply.send(do_pushback_feedback(&conn, &id, &status, decided_at, latency_ms));
            }
            Cmd::PushbacksSince { ts, reply } => {
                let _ = reply.send(do_pushbacks_since(&conn, ts));
            }
            Cmd::InsertApiCall { model, purpose, tokens_in, tokens_out, cost_usd, ok, error, reply } => {
                let _ = reply.send(do_insert_api_call(
                    &conn, clock.as_ref(), &model, &purpose,
                    tokens_in, tokens_out, cost_usd, ok, error.as_deref(),
                ));
            }
            Cmd::InsertDisclosure { api_call_id, model, purpose, memory_ids, observation_ids, reply } => {
                let _ = reply.send(do_insert_disclosure(
                    &conn, clock.as_ref(),
                    api_call_id.as_deref(), &model, &purpose, &memory_ids, &observation_ids,
                ));
            }
            Cmd::ClosedSessionsWithoutSummary { limit, reply } => {
                let _ = reply.send(do_closed_sessions_without_summary(&conn, limit));
            }
            Cmd::SetSessionSummary { id, summary, reply } => {
                let _ = reply.send(do_set_session_summary(&conn, &id, &summary));
            }
            Cmd::ObservationsBetween { project_id, from_ms, to_ms, limit, reply } => {
                let _ = reply.send(do_observations_between(&conn, &project_id, from_ms, to_ms, limit));
            }
            Cmd::DeleteObservationsOlderThan { cutoff_ms, protected_ids, max_rows, reply } => {
                let _ = reply.send(do_delete_observations_older_than(&conn, cutoff_ms, &protected_ids, max_rows));
            }
            // v4 — approvals
            Cmd::InsertApproval { new_approval, reply } => {
                let _ = reply.send(do_insert_approval(&conn, clock.as_ref(), new_approval));
            }
            Cmd::PendingApprovals { reply } => {
                let _ = reply.send(do_pending_approvals(&conn));
            }
            Cmd::GetApproval { id, reply } => {
                let _ = reply.send(do_get_approval(&conn, &id));
            }
            Cmd::DecideApproval { id, status, decided_at, decided_via, note, reply } => {
                let _ = reply.send(do_decide_approval(&conn, &id, &status, decided_at, &decided_via, note.as_deref()));
            }
            Cmd::SetApprovalExecution { id, execution, reply } => {
                let _ = reply.send(do_set_approval_execution(&conn, &id, &execution));
            }
            Cmd::ExpireApprovals { now_ms, reply } => {
                let _ = reply.send(do_expire_approvals(&conn, now_ms));
            }
            // v4 — agent_runs
            Cmd::InsertAgentRun { run, reply } => {
                let _ = reply.send(do_insert_agent_run(&conn, run));
            }
            Cmd::UpdateAgentRunStatus { id, status, ended, result_summary, diffstat, reply } => {
                let _ = reply.send(do_update_agent_run_status(&conn, &id, &status, ended, result_summary.as_deref(), diffstat.as_ref()));
            }
            Cmd::RecentAgentRuns { n, reply } => {
                let _ = reply.send(do_recent_agent_runs(&conn, n));
            }
            Cmd::GetAgentRun { id, reply } => {
                let _ = reply.send(do_get_agent_run(&conn, &id));
            }
            // v4 — blobs
            Cmd::InsertBlob { bytes, created, reply } => {
                let _ = reply.send(do_insert_blob(&conn, &bytes, created));
            }
            Cmd::GetBlob { id, reply } => {
                let _ = reply.send(do_get_blob(&conn, &id));
            }
        }
    }
}

fn do_upsert_project(
    conn: &Connection,
    clock: &dyn Clock,
    root_path: &str,
    name: &str,
) -> Result<Project, StoreError> {
    let now = clock.now_ms();
    let existing = conn
        .query_row(
            "SELECT id, root_path, name, first_seen, last_seen FROM projects WHERE root_path = ?1",
            params![root_path],
            |r| {
                Ok(Project {
                    id: r.get(0)?,
                    root_path: r.get(1)?,
                    name: r.get(2)?,
                    first_seen: r.get(3)?,
                    last_seen: r.get(4)?,
                })
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    match existing {
        Some(mut p) => {
            conn.execute("UPDATE projects SET last_seen = ?1 WHERE id = ?2", params![now, p.id])?;
            p.last_seen = now;
            Ok(p)
        }
        None => {
            let p = Project {
                id: new_id(),
                root_path: root_path.to_string(),
                name: name.to_string(),
                first_seen: now,
                last_seen: now,
            };
            conn.execute(
                "INSERT INTO projects (id, root_path, name, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![p.id, p.root_path, p.name, p.first_seen, p.last_seen],
            )?;
            Ok(p)
        }
    }
}

fn do_list_projects(conn: &Connection) -> Result<Vec<Project>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, root_path, name, first_seen, last_seen FROM projects ORDER BY last_seen DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Project {
            id: r.get(0)?,
            root_path: r.get(1)?,
            name: r.get(2)?,
            first_seen: r.get(3)?,
            last_seen: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn do_add_observation(
    conn: &Connection,
    clock: &dyn Clock,
    obs: NewObservation,
) -> Result<Observation, StoreError> {
    let observation = Observation {
        id: new_id(),
        event_id: obs.event_id,
        ts: clock.now_ms(),
        kind: obs.kind,
        project_id: obs.project_id,
        content: obs.content,
        meta: obs.meta,
    };
    conn.execute(
        "INSERT INTO observations (id, event_id, ts, kind, project_id, content, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            observation.id,
            observation.event_id,
            observation.ts,
            observation.kind,
            observation.project_id,
            observation.content,
            serde_json::to_string(&observation.meta)?
        ],
    )?;
    Ok(observation)
}

fn do_recent_observations(
    conn: &Connection,
    limit: u32,
    kind: Option<&str>,
) -> Result<Vec<Observation>, StoreError> {
    let sql = match kind {
        Some(_) => {
            "SELECT id, event_id, ts, kind, project_id, content, meta FROM observations
             WHERE kind = ?2 ORDER BY ts DESC, id DESC LIMIT ?1"
        }
        None => {
            "SELECT id, event_id, ts, kind, project_id, content, meta FROM observations
             ORDER BY ts DESC, id DESC LIMIT ?1"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let map = |r: &rusqlite::Row<'_>| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
        ))
    };
    let rows: Vec<_> = match kind {
        Some(k) => stmt.query_map(params![limit, k], map)?.collect::<Result<_, _>>()?,
        None => stmt.query_map(params![limit], map)?.collect::<Result<_, _>>()?,
    };
    let mut out = Vec::with_capacity(rows.len());
    for (id, event_id, ts, kind, project_id, content, meta) in rows {
        out.push(Observation {
            id,
            event_id,
            ts,
            kind,
            project_id,
            content,
            meta: serde_json::from_str(&meta)?,
        });
    }
    Ok(out)
}

fn do_session_open(conn: &Connection, ws: &WorkSession) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO work_sessions (id, project_id, started, last_activity, ended, commands)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![ws.id, ws.project_id, ws.started, ws.last_activity, ws.ended, ws.commands],
    )?;
    Ok(())
}

fn do_session_touch(
    conn: &Connection,
    id: &str,
    last_activity: i64,
    commands: u32,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE work_sessions SET last_activity = ?2, commands = ?3 WHERE id = ?1",
        params![id, last_activity, commands],
    )?;
    Ok(())
}

fn do_session_close(conn: &Connection, id: &str, ended: i64) -> Result<(), StoreError> {
    conn.execute("UPDATE work_sessions SET ended = ?2 WHERE id = ?1", params![id, ended])?;
    Ok(())
}

/// limit = Some(n): newest n sessions; None: only open (ended IS NULL) sessions.
fn do_sessions(conn: &Connection, limit: Option<u32>) -> Result<Vec<WorkSession>, StoreError> {
    let sql = match limit {
        Some(_) => {
            "SELECT id, project_id, started, last_activity, ended, commands FROM work_sessions
             ORDER BY started DESC LIMIT ?1"
        }
        None => {
            "SELECT id, project_id, started, last_activity, ended, commands FROM work_sessions
             WHERE ended IS NULL ORDER BY started DESC"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let map = |r: &rusqlite::Row<'_>| {
        Ok(WorkSession {
            id: r.get(0)?,
            project_id: r.get(1)?,
            started: r.get(2)?,
            last_activity: r.get(3)?,
            ended: r.get(4)?,
            commands: r.get(5)?,
        })
    };
    let rows: Vec<_> = match limit {
        Some(n) => stmt.query_map(params![n], map)?.collect::<Result<_, _>>()?,
        None => stmt.query_map([], map)?.collect::<Result<_, _>>()?,
    };
    Ok(rows)
}

fn do_append(conn: &Connection, clock: &dyn Clock, ev: NewEvent) -> Result<Event, StoreError> {
    let event = Event {
        id: new_id(),
        ts: clock.now_ms(),
        kind: ev.kind,
        source: ev.source,
        project_id: ev.project_id,
        session_id: ev.session_id,
        payload: ev.payload,
        lang: ev.lang,
    };
    conn.execute(
        "INSERT INTO events (id, ts, kind, source, project_id, session_id, payload, lang)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            event.id,
            event.ts,
            event.kind,
            event.source,
            event.project_id,
            event.session_id,
            serde_json::to_string(&event.payload)?,
            event.lang
        ],
    )?;
    Ok(event)
}

fn do_recent(conn: &Connection, limit: u32) -> Result<Vec<Event>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, kind, source, project_id, session_id, payload, lang
         FROM events ORDER BY ts DESC, id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;
    let mut events = Vec::new();
    for row in rows {
        let (id, ts, kind, source, project_id, session_id, payload, lang) = row?;
        events.push(Event {
            id,
            ts,
            kind,
            source,
            project_id,
            session_id,
            payload: serde_json::from_str(&payload)?,
            lang,
        });
    }
    Ok(events)
}

fn do_count(conn: &Connection) -> Result<u64, StoreError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
    Ok(n as u64)
}

// ---------------------------------------------------------------------------
// v3 implementation functions
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
type MemoryRow = (String, String, Option<String>, String, String, f64, i64, i64, String, i64);

#[allow(clippy::type_complexity)]
type PushbackRow = (
    String,
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    f64,
    String,
    Option<i64>,
    Option<i64>,
);

fn row_to_memory(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
    ))
}

fn tuple_to_memory(
    (id, r#type, project_id, title, body, confidence, created, updated, source_event_ids_s, archived): MemoryRow,
) -> Result<Memory, StoreError> {
    Ok(Memory {
        id,
        r#type,
        project_id,
        title,
        body,
        confidence,
        created,
        updated,
        source_event_ids: serde_json::from_str(&source_event_ids_s)?,
        archived: archived != 0,
    })
}

fn do_add_memory(conn: &Connection, clock: &dyn Clock, mem: NewMemory) -> Result<Memory, StoreError> {
    let now = clock.now_ms();
    let m = Memory {
        id: new_id(),
        r#type: mem.r#type,
        project_id: mem.project_id,
        title: mem.title,
        body: mem.body,
        confidence: mem.confidence,
        created: now,
        updated: now,
        source_event_ids: if mem.source_event_ids.is_null() {
            serde_json::json!([])
        } else {
            mem.source_event_ids
        },
        archived: false,
    };
    conn.execute(
        "INSERT INTO memories (id, type, project_id, title, body, confidence, created, updated, source_event_ids, archived)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            m.id, m.r#type, m.project_id, m.title, m.body, m.confidence,
            m.created, m.updated, serde_json::to_string(&m.source_event_ids)?, 0i64
        ],
    )?;
    Ok(m)
}

fn do_update_memory_confidence(conn: &Connection, clock: &dyn Clock, id: &str, confidence: f64) -> Result<(), StoreError> {
    let now = clock.now_ms();
    conn.execute(
        "UPDATE memories SET confidence = ?2, updated = ?3 WHERE id = ?1",
        params![id, confidence, now],
    )?;
    Ok(())
}

fn do_archive_memory(conn: &Connection, clock: &dyn Clock, id: &str) -> Result<(), StoreError> {
    let now = clock.now_ms();
    conn.execute(
        "UPDATE memories SET archived = 1, updated = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

fn do_list_memories(conn: &Connection, filter: &MemoryFilter) -> Result<Vec<Memory>, StoreError> {
    // Build WHERE clause and a parallel params list dynamically
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut idx = 1usize;

    if let Some(ref t) = filter.r#type {
        conditions.push(format!("type = ?{idx}"));
        params.push(Box::new(t.clone()));
        idx += 1;
    }
    if let Some(ref pid) = filter.project_id {
        conditions.push(format!("project_id = ?{idx}"));
        params.push(Box::new(pid.clone()));
        idx += 1;
    }
    let _ = idx; // suppress unused warning when both are None
    if !filter.include_archived {
        conditions.push("archived = 0".into());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    let sql = format!(
        "SELECT id, type, project_id, title, body, confidence, created, updated, source_event_ids, archived
         FROM memories {} ORDER BY updated DESC",
        where_clause
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<_> = stmt
        .query_map(rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())), row_to_memory)?
        .collect::<Result<_, _>>()?;
    rows.into_iter().map(tuple_to_memory).collect()
}

fn do_fts_observations(conn: &Connection, query: &str, limit: u32) -> Result<Vec<String>, StoreError> {
    // Join FTS results back to observations to get the real id; ORDER BY bm25 ASC is correct
    // because SQLite bm25() returns negative values (more negative = better match).
    let mut stmt = conn.prepare(
        "SELECT o.id
         FROM observations_fts f
         JOIN observations o ON o.rowid = f.rowid
         WHERE observations_fts MATCH ?1
         ORDER BY bm25(observations_fts)
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![query, limit], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn do_fts_memories(conn: &Connection, query: &str, limit: u32) -> Result<Vec<String>, StoreError> {
    // ORDER BY bm25 ASC is correct: bm25() is negative, most-negative = best match.
    let mut stmt = conn.prepare(
        "SELECT m.id
         FROM memories_fts f
         JOIN memories m ON m.rowid = f.rowid
         WHERE memories_fts MATCH ?1
         ORDER BY bm25(memories_fts)
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![query, limit], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn do_unembedded_observations(conn: &Connection, kinds: &[String], limit: u32) -> Result<Vec<Observation>, StoreError> {
    if kinds.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = kinds.iter().enumerate().map(|(i, _)| format!("?{}", i + 2)).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT o.id, o.event_id, o.ts, o.kind, o.project_id, o.content, o.meta
         FROM observations o
         LEFT JOIN vec_observations v ON v.obs_id = o.id
         WHERE v.obs_id IS NULL AND o.kind IN ({})
         ORDER BY o.ts ASC
         LIMIT ?1",
        placeholders
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(limit)];
    for k in kinds {
        all_params.push(Box::new(k.clone()));
    }
    let rows = stmt.query_map(
        rusqlite::params_from_iter(all_params.iter().map(|p| p.as_ref())),
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        let (id, event_id, ts, kind, project_id, content, meta) = row?;
        out.push(Observation {
            id,
            event_id,
            ts,
            kind,
            project_id,
            content,
            meta: serde_json::from_str(&meta)?,
        });
    }
    Ok(out)
}

fn do_set_observation_embedding(conn: &Connection, obs_id: &str, embedding: &[f32]) -> Result<(), StoreError> {
    let blob = encode_embedding(embedding);
    conn.execute(
        "INSERT INTO vec_observations (obs_id, embedding) VALUES (?1, ?2)
         ON CONFLICT(obs_id) DO UPDATE SET embedding = excluded.embedding",
        params![obs_id, blob],
    )?;
    Ok(())
}

fn do_set_memory_embedding(conn: &Connection, memory_id: &str, embedding: &[f32]) -> Result<(), StoreError> {
    let blob = encode_embedding(embedding);
    conn.execute(
        "INSERT INTO vec_memories (memory_id, embedding) VALUES (?1, ?2)
         ON CONFLICT(memory_id) DO UPDATE SET embedding = excluded.embedding",
        params![memory_id, blob],
    )?;
    Ok(())
}

fn do_all_observation_embeddings(conn: &Connection, limit: u32) -> Result<Vec<(String, Vec<f32>)>, StoreError> {
    let mut stmt = conn.prepare("SELECT obs_id, embedding FROM vec_observations LIMIT ?1")?;
    let rows = stmt.query_map(params![limit], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, blob) = row?;
        out.push((id, decode_embedding(&blob)));
    }
    Ok(out)
}

fn do_all_memory_embeddings(conn: &Connection, limit: u32) -> Result<Vec<(String, Vec<f32>)>, StoreError> {
    let mut stmt = conn.prepare("SELECT memory_id, embedding FROM vec_memories LIMIT ?1")?;
    let rows = stmt.query_map(params![limit], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, blob) = row?;
        out.push((id, decode_embedding(&blob)));
    }
    Ok(out)
}

fn do_observations_by_ids(conn: &Connection, ids: &[String]) -> Result<Vec<Observation>, StoreError> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT id, event_id, ts, kind, project_id, content, meta FROM observations WHERE id IN ({})",
        placeholders
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(ids.iter()),
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        let (id, event_id, ts, kind, project_id, content, meta) = row?;
        out.push(Observation {
            id,
            event_id,
            ts,
            kind,
            project_id,
            content,
            meta: serde_json::from_str(&meta)?,
        });
    }
    Ok(out)
}

fn row_to_pushback(r: &rusqlite::Row<'_>) -> rusqlite::Result<PushbackRow> {
    Ok((
        r.get(0)?,  // id
        r.get(1)?,  // ts
        r.get(2)?,  // mode
        r.get(3)?,  // trigger
        r.get(4)?,  // severity
        r.get(5)?,  // title
        r.get(6)?,  // message_en
        r.get(7)?,  // message_pt
        r.get(8)?,  // evidence (JSON TEXT)
        r.get(9)?,  // proposals (JSON TEXT)
        r.get(10)?, // confidence
        r.get(11)?, // status
        r.get(12)?, // decided_at
        r.get(13)?, // latency_ms
    ))
}

fn tuple_to_pushback(t: PushbackRow) -> Result<Pushback, StoreError> {
    Ok(Pushback {
        id: t.0,
        ts: t.1,
        mode: t.2,
        trigger: t.3,
        severity: t.4,
        title: t.5,
        message_en: t.6,
        message_pt: t.7,
        evidence: serde_json::from_str(&t.8)?,
        proposals: serde_json::from_str(&t.9)?,
        confidence: t.10,
        status: t.11,
        decided_at: t.12,
        latency_ms: t.13,
    })
}

fn do_insert_pushback(conn: &Connection, clock: &dyn Clock, pb: NewPushback) -> Result<Pushback, StoreError> {
    let now = clock.now_ms();
    let p = Pushback {
        id: new_id(),
        ts: now,
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
        decided_at: None,
        latency_ms: None,
    };
    conn.execute(
        "INSERT INTO pushbacks (id, ts, mode, trigger, severity, title, message_en, message_pt,
                                evidence, proposals, confidence, status, decided_at, latency_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            p.id, p.ts, p.mode, p.trigger, p.severity, p.title, p.message_en, p.message_pt,
            serde_json::to_string(&p.evidence)?,
            serde_json::to_string(&p.proposals)?,
            p.confidence, p.status, p.decided_at, p.latency_ms
        ],
    )?;
    Ok(p)
}

fn do_recent_pushbacks(conn: &Connection, limit: u32) -> Result<Vec<Pushback>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, mode, trigger, severity, title, message_en, message_pt,
                evidence, proposals, confidence, status, decided_at, latency_ms
         FROM pushbacks ORDER BY ts DESC LIMIT ?1",
    )?;
    let rows: Vec<_> = stmt.query_map(params![limit], row_to_pushback)?.collect::<Result<_, _>>()?;
    rows.into_iter().map(tuple_to_pushback).collect()
}

fn do_get_pushback(conn: &Connection, id: &str) -> Result<Option<Pushback>, StoreError> {
    let result = conn.query_row(
        "SELECT id, ts, mode, trigger, severity, title, message_en, message_pt,
                evidence, proposals, confidence, status, decided_at, latency_ms
         FROM pushbacks WHERE id = ?1",
        params![id],
        row_to_pushback,
    );
    match result {
        Ok(row) => Ok(Some(tuple_to_pushback(row)?)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StoreError::from(e)),
    }
}

fn do_pushback_feedback(conn: &Connection, id: &str, status: &str, decided_at: i64, latency_ms: i64) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE pushbacks SET status = ?2, decided_at = ?3, latency_ms = ?4 WHERE id = ?1",
        params![id, status, decided_at, latency_ms],
    )?;
    Ok(())
}

fn do_pushbacks_since(conn: &Connection, ts: i64) -> Result<Vec<Pushback>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, mode, trigger, severity, title, message_en, message_pt,
                evidence, proposals, confidence, status, decided_at, latency_ms
         FROM pushbacks WHERE ts >= ?1 ORDER BY ts ASC",
    )?;
    let rows: Vec<_> = stmt.query_map(params![ts], row_to_pushback)?.collect::<Result<_, _>>()?;
    rows.into_iter().map(tuple_to_pushback).collect()
}

#[allow(clippy::too_many_arguments)]
fn do_insert_api_call(
    conn: &Connection,
    clock: &dyn Clock,
    model: &str,
    purpose: &str,
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    cost_usd: Option<f64>,
    ok: bool,
    error: Option<&str>,
) -> Result<String, StoreError> {
    let id = new_id();
    let now = clock.now_ms();
    conn.execute(
        "INSERT INTO api_calls (id, ts, model, purpose, tokens_in, tokens_out, cost_usd, ok, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![id, now, model, purpose, tokens_in, tokens_out, cost_usd, ok as i64, error],
    )?;
    Ok(id)
}

fn do_insert_disclosure(
    conn: &Connection,
    clock: &dyn Clock,
    api_call_id: Option<&str>,
    model: &str,
    purpose: &str,
    memory_ids: &[String],
    observation_ids: &[String],
) -> Result<String, StoreError> {
    let id = new_id();
    let now = clock.now_ms();
    conn.execute(
        "INSERT INTO disclosures (id, ts, api_call_id, model, purpose, memory_ids, observation_ids)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id, now, api_call_id, model, purpose,
            serde_json::to_string(memory_ids)?,
            serde_json::to_string(observation_ids)?
        ],
    )?;
    Ok(id)
}

fn do_closed_sessions_without_summary(conn: &Connection, limit: u32) -> Result<Vec<WorkSession>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, started, last_activity, ended, commands
         FROM work_sessions
         WHERE ended IS NOT NULL AND summary IS NULL
         ORDER BY ended ASC
         LIMIT ?1",
    )?;
    let rows: Vec<_> = stmt.query_map(params![limit], |r| {
        Ok(WorkSession {
            id: r.get(0)?,
            project_id: r.get(1)?,
            started: r.get(2)?,
            last_activity: r.get(3)?,
            ended: r.get(4)?,
            commands: r.get(5)?,
        })
    })?.collect::<Result<_, _>>()?;
    Ok(rows)
}

fn do_set_session_summary(conn: &Connection, id: &str, summary: &str) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE work_sessions SET summary = ?2 WHERE id = ?1",
        params![id, summary],
    )?;
    Ok(())
}

fn do_observations_between(
    conn: &Connection,
    project_id: &str,
    from_ms: i64,
    to_ms: i64,
    limit: u32,
) -> Result<Vec<Observation>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, event_id, ts, kind, project_id, content, meta
         FROM observations
         WHERE project_id = ?1 AND ts >= ?2 AND ts <= ?3
         ORDER BY ts ASC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![project_id, from_ms, to_ms, limit], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, event_id, ts, kind, project_id, content, meta) = row?;
        out.push(Observation {
            id,
            event_id,
            ts,
            kind,
            project_id,
            content,
            meta: serde_json::from_str(&meta)?,
        });
    }
    Ok(out)
}

fn do_delete_observations_older_than(
    conn: &Connection,
    cutoff_ms: i64,
    protected_ids: &[String],
    max_rows: u32,
) -> Result<u32, StoreError> {
    // Build a subquery to exclude protected ids
    let placeholders = if protected_ids.is_empty() {
        String::new()
    } else {
        let ph = protected_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 3))
            .collect::<Vec<_>>()
            .join(", ");
        format!("AND id NOT IN ({})", ph)
    };

    let select_sql = format!(
        "SELECT id FROM observations WHERE ts < ?1 {} LIMIT ?2",
        placeholders
    );
    let mut select_params: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(cutoff_ms),
        Box::new(max_rows),
    ];
    for id in protected_ids {
        select_params.push(Box::new(id.clone()));
    }

    let mut stmt = conn.prepare(&select_sql)?;
    let ids_to_delete: Vec<String> = stmt
        .query_map(
            rusqlite::params_from_iter(select_params.iter().map(|p| p.as_ref())),
            |r| r.get(0),
        )?
        .collect::<Result<_, _>>()?;

    if ids_to_delete.is_empty() {
        return Ok(0);
    }

    let count = ids_to_delete.len() as u32;

    // Delete vec_observations first
    for id in &ids_to_delete {
        conn.execute("DELETE FROM vec_observations WHERE obs_id = ?1", params![id])?;
    }

    // Delete the observations (FTS goes via trigger)
    let del_phs = ids_to_delete
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let del_sql = format!("DELETE FROM observations WHERE id IN ({})", del_phs);
    conn.execute(
        &del_sql,
        rusqlite::params_from_iter(ids_to_delete.iter()),
    )?;

    Ok(count)
}

// ---------------------------------------------------------------------------
// v4 implementation functions — approvals
// ---------------------------------------------------------------------------

fn row_to_approval(r: &rusqlite::Row<'_>) -> rusqlite::Result<ApprovalRow> {
    Ok((
        r.get(0)?,  // id
        r.get(1)?,  // created
        r.get(2)?,  // kind
        r.get(3)?,  // risk
        r.get(4)?,  // title
        r.get(5)?,  // reason
        r.get(6)?,  // cwd
        r.get(7)?,  // target
        r.get(8)?,  // agent_identity
        r.get(9)?,  // payload (JSON text)
        r.get(10)?, // expected_impact (JSON text)
        r.get(11)?, // expires_at
        r.get(12)?, // status
        r.get(13)?, // decided_at
        r.get(14)?, // decided_via
        r.get(15)?, // decision_note
        r.get(16)?, // execution (JSON text or NULL)
    ))
}

#[allow(clippy::type_complexity)]
type ApprovalRow = (
    String, i64, String, i64, String, String,
    Option<String>, Option<String>, String, String, String, i64, String,
    Option<i64>, Option<String>, Option<String>, Option<String>,
);

fn tuple_to_approval(t: ApprovalRow) -> Result<Approval, StoreError> {
    Ok(Approval {
        id: t.0,
        created: t.1,
        kind: t.2,
        risk: t.3,
        title: t.4,
        reason: t.5,
        cwd: t.6,
        target: t.7,
        agent_identity: t.8,
        payload: serde_json::from_str(&t.9)?,
        expected_impact: serde_json::from_str(&t.10)?,
        expires_at: t.11,
        status: t.12,
        decided_at: t.13,
        decided_via: t.14,
        decision_note: t.15,
        execution: t.16.map(|s| serde_json::from_str(&s)).transpose()?,
    })
}

const APPROVAL_SELECT: &str =
    "SELECT id, created, kind, risk, title, reason, cwd, target, agent_identity,
            payload, expected_impact, expires_at, status, decided_at, decided_via,
            decision_note, execution FROM approvals";

fn do_insert_approval(
    conn: &Connection,
    clock: &dyn Clock,
    na: NewApproval,
) -> Result<Approval, StoreError> {
    let now = clock.now_ms();
    let a = Approval {
        id: new_id(),
        created: now,
        kind: na.kind,
        risk: na.risk,
        title: na.title,
        reason: na.reason,
        cwd: na.cwd,
        target: na.target,
        agent_identity: na.agent_identity,
        payload: na.payload,
        expected_impact: na.expected_impact,
        expires_at: na.expires_at,
        status: "pending".into(),
        decided_at: None,
        decided_via: None,
        decision_note: None,
        execution: None,
    };
    conn.execute(
        "INSERT INTO approvals (id, created, kind, risk, title, reason, cwd, target,
                                agent_identity, payload, expected_impact, expires_at,
                                status, decided_at, decided_via, decision_note, execution)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            a.id, a.created, a.kind, a.risk, a.title, a.reason, a.cwd, a.target,
            a.agent_identity,
            serde_json::to_string(&a.payload)?,
            serde_json::to_string(&a.expected_impact)?,
            a.expires_at, a.status, a.decided_at, a.decided_via, a.decision_note,
            a.execution.as_ref().map(serde_json::to_string).transpose()?
        ],
    )?;
    Ok(a)
}

fn do_pending_approvals(conn: &Connection) -> Result<Vec<Approval>, StoreError> {
    let sql = format!("{} WHERE status = 'pending' ORDER BY created ASC", APPROVAL_SELECT);
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<ApprovalRow> = stmt
        .query_map([], row_to_approval)?
        .collect::<Result<_, _>>()?;
    rows.into_iter().map(tuple_to_approval).collect()
}

fn do_get_approval(conn: &Connection, id: &str) -> Result<Option<Approval>, StoreError> {
    let sql = format!("{} WHERE id = ?1", APPROVAL_SELECT);
    match conn.query_row(&sql, params![id], row_to_approval) {
        Ok(row) => Ok(Some(tuple_to_approval(row)?)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StoreError::from(e)),
    }
}

fn do_decide_approval(
    conn: &Connection,
    id: &str,
    status: &str,
    decided_at: i64,
    decided_via: &str,
    note: Option<&str>,
) -> Result<Approval, StoreError> {
    // Fetch current row — must exist and must be pending
    let current = do_get_approval(conn, id)?
        .ok_or(StoreError::NotFound)?;
    if current.status != "pending" {
        return Err(StoreError::InvalidState(format!(
            "approval {} is '{}', not 'pending'; cannot decide",
            id, current.status
        )));
    }
    conn.execute(
        "UPDATE approvals SET status = ?2, decided_at = ?3, decided_via = ?4, decision_note = ?5
         WHERE id = ?1",
        params![id, status, decided_at, decided_via, note],
    )?;
    // Return updated row
    do_get_approval(conn, id)?.ok_or(StoreError::NotFound)
}

fn do_set_approval_execution(
    conn: &Connection,
    id: &str,
    execution: &serde_json::Value,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE approvals SET execution = ?2 WHERE id = ?1",
        params![id, serde_json::to_string(execution)?],
    )?;
    Ok(())
}

fn do_expire_approvals(conn: &Connection, now_ms: i64) -> Result<u32, StoreError> {
    let n = conn.execute(
        "UPDATE approvals SET status = 'expired'
         WHERE status = 'pending' AND expires_at <= ?1",
        params![now_ms],
    )?;
    Ok(n as u32)
}

// ---------------------------------------------------------------------------
// v4 implementation functions — agent_runs
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
type AgentRunRow = (
    String, String, String, String, String, String, Option<String>,
    String, String, String, f64, i64, Option<i64>, Option<String>, Option<String>,
);

fn row_to_agent_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRunRow> {
    Ok((
        r.get(0)?,  // id
        r.get(1)?,  // adapter
        r.get(2)?,  // task_title
        r.get(3)?,  // project_id
        r.get(4)?,  // worktree_path
        r.get(5)?,  // branch
        r.get(6)?,  // tmux_target
        r.get(7)?,  // mode
        r.get(8)?,  // status
        r.get(9)?,  // tokens (JSON text)
        r.get(10)?, // cost_usd
        r.get(11)?, // started
        r.get(12)?, // ended
        r.get(13)?, // result_summary
        r.get(14)?, // diffstat (JSON text or NULL)
    ))
}

fn tuple_to_agent_run(t: AgentRunRow) -> Result<AgentRun, StoreError> {
    Ok(AgentRun {
        id: t.0,
        adapter: t.1,
        task_title: t.2,
        project_id: t.3,
        worktree_path: t.4,
        branch: t.5,
        tmux_target: t.6,
        mode: t.7,
        status: t.8,
        tokens: serde_json::from_str(&t.9)?,
        cost_usd: t.10,
        started: t.11,
        ended: t.12,
        result_summary: t.13,
        diffstat: t.14.map(|s| serde_json::from_str(&s)).transpose()?,
    })
}

const AGENT_RUN_SELECT: &str =
    "SELECT id, adapter, task_title, project_id, worktree_path, branch, tmux_target,
            mode, status, tokens, cost_usd, started, ended, result_summary, diffstat
     FROM agent_runs";

fn do_insert_agent_run(conn: &Connection, run: NewAgentRun) -> Result<AgentRun, StoreError> {
    let r = AgentRun {
        id: new_id(),
        adapter: run.adapter,
        task_title: run.task_title,
        project_id: run.project_id,
        worktree_path: run.worktree_path,
        branch: run.branch,
        tmux_target: run.tmux_target,
        mode: run.mode,
        status: "running".into(),
        tokens: run.tokens,
        cost_usd: run.cost_usd,
        started: run.started,
        ended: None,
        result_summary: None,
        diffstat: None,
    };
    conn.execute(
        "INSERT INTO agent_runs (id, adapter, task_title, project_id, worktree_path, branch,
                                  tmux_target, mode, status, tokens, cost_usd, started, ended,
                                  result_summary, diffstat)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            r.id, r.adapter, r.task_title, r.project_id, r.worktree_path, r.branch,
            r.tmux_target, r.mode, r.status,
            serde_json::to_string(&r.tokens)?,
            r.cost_usd, r.started, r.ended, r.result_summary,
            r.diffstat.as_ref().map(serde_json::to_string).transpose()?
        ],
    )?;
    Ok(r)
}

fn do_update_agent_run_status(
    conn: &Connection,
    id: &str,
    status: &str,
    ended: Option<i64>,
    result_summary: Option<&str>,
    diffstat: Option<&serde_json::Value>,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE agent_runs SET status = ?2, ended = ?3, result_summary = ?4, diffstat = ?5
         WHERE id = ?1",
        params![
            id, status, ended, result_summary,
            diffstat.map(serde_json::to_string).transpose()?
        ],
    )?;
    Ok(())
}

fn do_recent_agent_runs(conn: &Connection, n: u32) -> Result<Vec<AgentRun>, StoreError> {
    let sql = format!("{} ORDER BY started DESC LIMIT ?1", AGENT_RUN_SELECT);
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<AgentRunRow> = stmt
        .query_map(params![n], row_to_agent_run)?
        .collect::<Result<_, _>>()?;
    rows.into_iter().map(tuple_to_agent_run).collect()
}

fn do_get_agent_run(conn: &Connection, id: &str) -> Result<Option<AgentRun>, StoreError> {
    let sql = format!("{} WHERE id = ?1", AGENT_RUN_SELECT);
    match conn.query_row(&sql, params![id], row_to_agent_run) {
        Ok(row) => Ok(Some(tuple_to_agent_run(row)?)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StoreError::from(e)),
    }
}

// ---------------------------------------------------------------------------
// v4 implementation functions — blobs
// ---------------------------------------------------------------------------

fn do_insert_blob(conn: &Connection, bytes: &[u8], created: i64) -> Result<Blob, StoreError> {
    let digest = sha256_hex(bytes);

    // Check for existing blob with same sha256
    let existing = conn
        .query_row(
            "SELECT id, sha256, bytes, created FROM blobs WHERE sha256 = ?1",
            params![digest],
            |r| {
                Ok(Blob {
                    id: r.get(0)?,
                    sha256: r.get(1)?,
                    bytes: r.get(2)?,
                    created: r.get(3)?,
                })
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;

    if let Some(blob) = existing {
        return Ok(blob);
    }

    let blob = Blob {
        id: new_id(),
        sha256: digest,
        bytes: bytes.to_vec(),
        created,
    };
    conn.execute(
        "INSERT INTO blobs (id, sha256, bytes, created) VALUES (?1, ?2, ?3, ?4)",
        params![blob.id, blob.sha256, blob.bytes, blob.created],
    )?;
    Ok(blob)
}

fn do_get_blob(conn: &Connection, id: &str) -> Result<Option<Blob>, StoreError> {
    match conn.query_row(
        "SELECT id, sha256, bytes, created FROM blobs WHERE id = ?1",
        params![id],
        |r| {
            Ok(Blob {
                id: r.get(0)?,
                sha256: r.get(1)?,
                bytes: r.get(2)?,
                created: r.get(3)?,
            })
        },
    ) {
        Ok(blob) => Ok(Some(blob)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StoreError::from(e)),
    }
}

/// Compute the SHA-256 digest of `data` and return it as a 64-char hex string.
fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut s = String::with_capacity(64);
    for b in digest.iter() {
        use std::fmt::Write;
        write!(s, "{:02x}", b).unwrap();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use rat_core::clock::FakeClock;
    use rat_proto::NewEvent;

    fn ev(kind: &str) -> NewEvent {
        NewEvent { kind: kind.into(), source: "test".into(), ..Default::default() }
    }

    #[tokio::test]
    async fn append_assigns_id_and_clock_ts() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();
        let e = store.append(ev("a")).await.unwrap();
        assert_eq!(e.ts, 1_000);
        assert_eq!(e.id.len(), 26);
    }

    #[tokio::test]
    async fn recent_returns_newest_first_and_count_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();
        store.append(ev("first")).await.unwrap();
        clock.advance(10);
        store.append(ev("second")).await.unwrap();
        let recent = store.recent(10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].kind, "second");
        assert_eq!(store.count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn events_persist_across_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        {
            let store = Store::open(&path, FakeClock::at(1)).unwrap();
            store.append(ev("kept")).await.unwrap();
        }
        let store = Store::open(&path, FakeClock::at(2)).unwrap();
        assert_eq!(store.count().await.unwrap(), 1);
        assert_eq!(store.recent(1).await.unwrap()[0].kind, "kept");
    }

    #[tokio::test]
    async fn project_upsert_is_idempotent_by_root_path() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(100);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();
        let a = store.upsert_project("/home/u/proj".into(), "proj".into()).await.unwrap();
        clock.advance(50);
        let b = store.upsert_project("/home/u/proj".into(), "proj".into()).await.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.first_seen, 100);
        assert_eq!(b.last_seen, 150);
        assert_eq!(store.list_projects().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn session_lifecycle_open_touch_close() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(0)).unwrap();
        let ws = rat_proto::WorkSession {
            id: "s1".into(),
            project_id: "p1".into(),
            started: 10,
            last_activity: 10,
            ended: None,
            commands: 1,
        };
        store.session_open(ws).await.unwrap();
        assert_eq!(store.open_sessions().await.unwrap().len(), 1);
        store.session_touch("s1".into(), 20, 3).await.unwrap();
        let got = &store.recent_sessions(10).await.unwrap()[0];
        assert_eq!(got.last_activity, 20);
        assert_eq!(got.commands, 3);
        store.session_close("s1".into(), 20).await.unwrap();
        assert_eq!(store.open_sessions().await.unwrap().len(), 0);
        assert_eq!(store.recent_sessions(10).await.unwrap()[0].ended, Some(20));
    }

    #[tokio::test]
    async fn observation_round_trips_with_meta_and_kind_filter() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(5)).unwrap();
        store
            .add_observation(rat_proto::NewObservation {
                kind: "shell_cmd".into(),
                content: "cargo test".into(),
                meta: serde_json::json!({"exit": 0}),
                ..Default::default()
            })
            .await
            .unwrap();
        store
            .add_observation(rat_proto::NewObservation {
                kind: "clipboard_text".into(),
                content: "hi".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let all = store.recent_observations(10, None).await.unwrap();
        assert_eq!(all.len(), 2);
        let shell = store.recent_observations(10, Some("shell_cmd".into())).await.unwrap();
        assert_eq!(shell.len(), 1);
        assert_eq!(shell[0].content, "cargo test");
        assert_eq!(shell[0].meta["exit"], 0);
    }

    #[tokio::test]
    async fn payload_round_trips_as_json() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1)).unwrap();
        let mut e = ev("p");
        e.payload = serde_json::json!({"n": 1, "s": "x"});
        store.append(e).await.unwrap();
        let got = &store.recent(1).await.unwrap()[0];
        assert_eq!(got.payload["n"], 1);
        assert_eq!(got.payload["s"], "x");
    }

    // -----------------------------------------------------------------------
    // v3 tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn memory_insert_list_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        let m = store
            .add_memory(crate::rows::NewMemory {
                r#type: "episode_summary".into(),
                project_id: Some("p1".into()),
                title: "Fixed the build".into(),
                body: "We fixed all clippy warnings".into(),
                confidence: 0.9,
                source_event_ids: serde_json::json!(["e1", "e2"]),
            })
            .await
            .unwrap();
        assert_eq!(m.r#type, "episode_summary");
        assert_eq!(m.created, 1_000);
        assert!(!m.archived);

        // list unarchived
        let all = store
            .list_memories(crate::rows::MemoryFilter {
                include_archived: false,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(all.len(), 1);

        // archive it
        store.archive_memory(m.id.clone()).await.unwrap();
        let non_archived = store
            .list_memories(crate::rows::MemoryFilter::default())
            .await
            .unwrap();
        assert!(non_archived.is_empty());

        let with_archived = store
            .list_memories(crate::rows::MemoryFilter { include_archived: true, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(with_archived.len(), 1);
        assert!(with_archived[0].archived);
    }

    #[tokio::test]
    async fn fts_trigger_sync_insert_and_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1)).unwrap();

        // Insert an observation — trigger should populate FTS
        let obs = store
            .add_observation(rat_proto::NewObservation {
                kind: "shell_cmd".into(),
                content: "cargo test --workspace".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        // FTS should find it
        let hits = store.fts_observations("cargo".into(), 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0], obs.id);

        // Now add a memory and search it
        let mem = store
            .add_memory(crate::rows::NewMemory {
                r#type: "note".into(),
                title: "Deployment procedure".into(),
                body: "Run make deploy before pushing".into(),
                confidence: 0.8,
                ..Default::default()
            })
            .await
            .unwrap();

        let mem_hits = store.fts_memories("deploy".into(), 10).await.unwrap();
        assert_eq!(mem_hits.len(), 1);
        assert_eq!(mem_hits[0], mem.id);
    }

    #[tokio::test]
    async fn observation_embedding_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1)).unwrap();

        let obs = store
            .add_observation(rat_proto::NewObservation {
                kind: "shell_cmd".into(),
                content: "hello".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let embedding: Vec<f32> = vec![0.1, 0.2, -0.3, 1.0];
        store.set_observation_embedding(obs.id.clone(), embedding.clone()).await.unwrap();

        let all = store.all_observation_embeddings(10).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, obs.id);
        assert_eq!(all[0].1, embedding);
    }

    #[tokio::test]
    async fn unembedded_observations_filters_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1)).unwrap();

        let obs1 = store
            .add_observation(rat_proto::NewObservation {
                kind: "shell_cmd".into(),
                content: "cmd1".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        store
            .add_observation(rat_proto::NewObservation {
                kind: "clipboard_text".into(),
                content: "clip".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        // Only shell_cmd without embedding
        let unembedded = store
            .unembedded_observations(vec!["shell_cmd".into()], 10)
            .await
            .unwrap();
        assert_eq!(unembedded.len(), 1);

        // Set embedding, now it should not appear
        store.set_observation_embedding(obs1.id.clone(), vec![1.0, 0.0]).await.unwrap();
        let unembedded2 = store
            .unembedded_observations(vec!["shell_cmd".into()], 10)
            .await
            .unwrap();
        assert!(unembedded2.is_empty());
    }

    #[tokio::test]
    async fn pushback_insert_feedback_status_transition() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(5_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        let pb = store
            .insert_pushback(crate::rows::NewPushback {
                mode: "mentor".into(),
                trigger: "stuck_loop".into(),
                severity: "warn".into(),
                title: "Repeating failed command".into(),
                message_en: "You ran the same command 3 times.".into(),
                message_pt: "Você executou o mesmo comando 3 vezes.".into(),
                evidence: serde_json::json!([{"observation_id": "obs1", "quote": "cargo build"}]),
                proposals: serde_json::json!([{"kind": "suggest", "detail": "Check the error"}]),
                confidence: 0.85,
                status: "shown".into(),
            })
            .await
            .unwrap();

        assert_eq!(pb.status, "shown");
        assert_eq!(pb.ts, 5_000);

        // list recent
        let recent = store.recent_pushbacks(10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, pb.id);

        // pushbacks_since
        let since = store.pushbacks_since(1_000).await.unwrap();
        assert_eq!(since.len(), 1);
        let empty = store.pushbacks_since(6_000).await.unwrap();
        assert!(empty.is_empty());

        // feedback transition
        clock.advance(1_000);
        store
            .pushback_feedback(pb.id.clone(), "dismissed".into(), 6_000, 1_000)
            .await
            .unwrap();

        let updated = store.recent_pushbacks(10).await.unwrap();
        assert_eq!(updated[0].status, "dismissed");
        assert_eq!(updated[0].decided_at, Some(6_000));
        assert_eq!(updated[0].latency_ms, Some(1_000));
    }

    #[tokio::test]
    async fn closed_sessions_without_summary() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(0)).unwrap();

        // open + close a session (no summary)
        let ws = rat_proto::WorkSession {
            id: "s1".into(),
            project_id: "p1".into(),
            started: 1,
            last_activity: 10,
            ended: None,
            commands: 2,
        };
        store.session_open(ws).await.unwrap();
        store.session_close("s1".into(), 20).await.unwrap();

        let without = store.closed_sessions_without_summary(10).await.unwrap();
        assert_eq!(without.len(), 1);
        assert_eq!(without[0].id, "s1");

        // set summary
        store.set_session_summary("s1".into(), "Did some work".into()).await.unwrap();

        let after = store.closed_sessions_without_summary(10).await.unwrap();
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn api_call_and_disclosure_insert() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1_000)).unwrap();

        let call_id = store
            .insert_api_call(
                "gpt-4".into(),
                "critic".into(),
                Some(100),
                Some(50),
                Some(0.002),
                true,
                None,
            )
            .await
            .unwrap();
        assert_eq!(call_id.len(), 26);

        let disc_id = store
            .insert_disclosure(
                Some(call_id.clone()),
                "gpt-4".into(),
                "critic".into(),
                vec!["m1".into()],
                vec!["o1".into(), "o2".into()],
            )
            .await
            .unwrap();
        assert_eq!(disc_id.len(), 26);
    }

    #[tokio::test]
    async fn observations_between_returns_time_window() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        // obs at t=1000
        store.add_observation(rat_proto::NewObservation {
            kind: "shell_cmd".into(),
            content: "cmd at 1000".into(),
            project_id: Some("p1".into()),
            ..Default::default()
        }).await.unwrap();

        clock.advance(2_000); // t=3000
        store.add_observation(rat_proto::NewObservation {
            kind: "shell_cmd".into(),
            content: "cmd at 3000".into(),
            project_id: Some("p1".into()),
            ..Default::default()
        }).await.unwrap();

        clock.advance(2_000); // t=5000
        store.add_observation(rat_proto::NewObservation {
            kind: "shell_cmd".into(),
            content: "cmd at 5000".into(),
            project_id: Some("p1".into()),
            ..Default::default()
        }).await.unwrap();

        // Between 500 and 4000 → should return 2
        let result = store.observations_between("p1", 500, 4000, 10).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "cmd at 1000");
        assert_eq!(result[1].content, "cmd at 3000");

        // Different project → none
        let result2 = store.observations_between("other", 0, 10_000, 10).await.unwrap();
        assert!(result2.is_empty());
    }

    #[tokio::test]
    async fn delete_observations_older_than_respects_protected() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        let obs_old = store.add_observation(rat_proto::NewObservation {
            kind: "shell_cmd".into(),
            content: "old cmd".into(),
            ..Default::default()
        }).await.unwrap();

        let obs_protected = store.add_observation(rat_proto::NewObservation {
            kind: "shell_cmd".into(),
            content: "protected old cmd".into(),
            ..Default::default()
        }).await.unwrap();

        clock.advance(10_000);
        let obs_new = store.add_observation(rat_proto::NewObservation {
            kind: "shell_cmd".into(),
            content: "new cmd".into(),
            ..Default::default()
        }).await.unwrap();

        // Store an embedding for obs_old to verify vec cleanup
        store.set_observation_embedding(obs_old.id.clone(), vec![1.0, 0.0]).await.unwrap();

        // Delete older than t=5000, protecting obs_protected
        let deleted = store
            .delete_observations_older_than(5_000, std::slice::from_ref(&obs_protected.id), 100)
            .await
            .unwrap();
        assert_eq!(deleted, 1); // only obs_old

        let remaining = store.recent_observations(10, None).await.unwrap();
        assert_eq!(remaining.len(), 2);
        let ids: Vec<&str> = remaining.iter().map(|o| o.id.as_str()).collect();
        assert!(ids.contains(&obs_protected.id.as_str()));
        assert!(ids.contains(&obs_new.id.as_str()));

        // vec_observations row should be gone
        let embs = store.all_observation_embeddings(10).await.unwrap();
        assert!(embs.is_empty());
    }

    // -----------------------------------------------------------------------
    // v4 tests — approvals
    // -----------------------------------------------------------------------

    fn new_approval_fixture(expires_at: i64) -> crate::rows::NewApproval {
        crate::rows::NewApproval {
            kind: "command".into(),
            risk: 1,
            title: "Run cargo build".into(),
            reason: "Build the project".into(),
            cwd: Some("/home/user/proj".into()),
            target: None,
            agent_identity: "test-agent".into(),
            payload: serde_json::json!({"cmd": "cargo build"}),
            expected_impact: serde_json::json!({"files": []}),
            expires_at,
        }
    }

    #[tokio::test]
    async fn approval_insert_and_pending_list() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        let a = store.insert_approval(new_approval_fixture(9_000)).await.unwrap();
        assert_eq!(a.status, "pending");
        assert_eq!(a.created, 1_000);
        assert_eq!(a.kind, "command");
        assert_eq!(a.id.len(), 26);

        let pending = store.pending_approvals().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, a.id);

        let got = store.get_approval(a.id.clone()).await.unwrap();
        assert!(got.is_some());
        assert_eq!(got.unwrap().title, "Run cargo build");
    }

    #[tokio::test]
    async fn approval_decide_approved_denied() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        let a = store.insert_approval(new_approval_fixture(9_000)).await.unwrap();

        // Approve it
        clock.advance(500);
        let updated = store
            .decide_approval(a.id.clone(), "approved".into(), 1_500, "popup".into(), None)
            .await
            .unwrap();
        assert_eq!(updated.status, "approved");
        assert_eq!(updated.decided_at, Some(1_500));
        assert_eq!(updated.decided_via.as_deref(), Some("popup"));

        // No longer in pending list
        let pending = store.pending_approvals().await.unwrap();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn approval_double_decide_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        let a = store.insert_approval(new_approval_fixture(9_000)).await.unwrap();

        // First decide: approved
        store
            .decide_approval(a.id.clone(), "approved".into(), 1_001, "popup".into(), None)
            .await
            .unwrap();

        // Second decide on non-pending row must fail
        let err = store
            .decide_approval(a.id.clone(), "denied".into(), 1_002, "popup".into(), None)
            .await;
        assert!(err.is_err(), "deciding a non-pending approval must return an error");
        match err.unwrap_err() {
            crate::error::StoreError::InvalidState(_) => {}
            other => panic!("expected InvalidState, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn approval_expiry() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        // expires_at = 2_000; now = 1_000 → not yet expired
        let _a1 = store.insert_approval(new_approval_fixture(2_000)).await.unwrap();
        // expires_at = 500; now = 1_000 → should expire
        let a2 = store.insert_approval(new_approval_fixture(500)).await.unwrap();

        let expired_count = store.expire_approvals(1_000).await.unwrap();
        assert_eq!(expired_count, 1);

        let a2_got = store.get_approval(a2.id.clone()).await.unwrap().unwrap();
        assert_eq!(a2_got.status, "expired");

        // The other one still pending
        let pending = store.pending_approvals().await.unwrap();
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn approval_set_execution() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        let a = store.insert_approval(new_approval_fixture(9_000)).await.unwrap();
        store
            .decide_approval(a.id.clone(), "approved".into(), 1_001, "cli".into(), Some("auto".into()))
            .await
            .unwrap();

        let exec = serde_json::json!({"started": 1_100, "exit_code": 0, "output_ref": null});
        store.set_approval_execution(a.id.clone(), exec.clone()).await.unwrap();

        let got = store.get_approval(a.id.clone()).await.unwrap().unwrap();
        assert!(got.execution.is_some());
        assert_eq!(got.execution.unwrap()["exit_code"], 0);
    }

    // -----------------------------------------------------------------------
    // v4 tests — agent_runs
    // -----------------------------------------------------------------------

    fn new_run_fixture(started: i64) -> crate::rows::NewAgentRun {
        crate::rows::NewAgentRun {
            adapter: "test-adapter".into(),
            task_title: "Fix the bug".into(),
            project_id: "proj-1".into(),
            worktree_path: "/tmp/wt/fix".into(),
            branch: "fix/bug-123".into(),
            tmux_target: Some("rat:workbench.0".into()),
            mode: "headless".into(),
            tokens: serde_json::json!({"in": 100, "out": 50}),
            cost_usd: 0.001,
            started,
        }
    }

    #[tokio::test]
    async fn agent_run_insert_and_status_update() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(5_000)).unwrap();

        let run = store.insert_agent_run(new_run_fixture(5_000)).await.unwrap();
        assert_eq!(run.status, "running");
        assert_eq!(run.adapter, "test-adapter");
        assert_eq!(run.id.len(), 26);
        assert!(run.ended.is_none());

        store
            .update_agent_run_status(
                run.id.clone(),
                "done".into(),
                Some(6_000),
                Some("Fixed the bug successfully".into()),
                Some(serde_json::json!({"added": 10, "removed": 2})),
            )
            .await
            .unwrap();

        let got = store.get_agent_run(run.id.clone()).await.unwrap().unwrap();
        assert_eq!(got.status, "done");
        assert_eq!(got.ended, Some(6_000));
        assert_eq!(got.result_summary.as_deref(), Some("Fixed the bug successfully"));
        assert!(got.diffstat.is_some());
        assert_eq!(got.diffstat.unwrap()["added"], 10);
    }

    #[tokio::test]
    async fn agent_run_recent_returns_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1_000)).unwrap();

        store.insert_agent_run(new_run_fixture(1_000)).await.unwrap();
        store.insert_agent_run(new_run_fixture(2_000)).await.unwrap();
        store.insert_agent_run(new_run_fixture(3_000)).await.unwrap();

        let recent = store.recent_agent_runs(2).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].started, 3_000);
        assert_eq!(recent[1].started, 2_000);
    }

    #[tokio::test]
    async fn agent_run_get_nonexistent_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1_000)).unwrap();
        let got = store.get_agent_run("nonexistent-id".into()).await.unwrap();
        assert!(got.is_none());
    }

    // -----------------------------------------------------------------------
    // v4 tests — blobs
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn blob_insert_and_get() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1_000)).unwrap();

        let data = b"hello, world!";
        let blob = store.insert_blob(data.to_vec(), 1_000).await.unwrap();
        assert_eq!(blob.id.len(), 26);
        assert_eq!(blob.bytes, data);
        assert_eq!(blob.sha256.len(), 64);
        assert_eq!(blob.created, 1_000);

        let got = store.get_blob(blob.id.clone()).await.unwrap();
        assert!(got.is_some());
        let got = got.unwrap();
        assert_eq!(got.bytes, data.to_vec());
        assert_eq!(got.sha256, blob.sha256);
    }

    #[tokio::test]
    async fn blob_deduplication_on_sha256() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1_000)).unwrap();

        let data = b"deduplicate me";
        let b1 = store.insert_blob(data.to_vec(), 1_000).await.unwrap();
        let b2 = store.insert_blob(data.to_vec(), 2_000).await.unwrap(); // same content, different time

        // Must return the same id and sha256
        assert_eq!(b1.id, b2.id, "same content must deduplicate to same id");
        assert_eq!(b1.sha256, b2.sha256);
        assert_eq!(b1.created, b2.created, "deduplicated blob should keep original created");

        // Only one row in db
        let got = store.get_blob(b1.id.clone()).await.unwrap().unwrap();
        assert_eq!(got.id, b1.id);
    }

    #[tokio::test]
    async fn blob_different_content_different_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1_000)).unwrap();

        let b1 = store.insert_blob(b"content-a".to_vec(), 1_000).await.unwrap();
        let b2 = store.insert_blob(b"content-b".to_vec(), 1_000).await.unwrap();
        assert_ne!(b1.id, b2.id);
        assert_ne!(b1.sha256, b2.sha256);
    }

    #[tokio::test]
    async fn blob_get_nonexistent_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db"), FakeClock::at(1_000)).unwrap();
        let got = store.get_blob("nonexistent".into()).await.unwrap();
        assert!(got.is_none());
    }
}
