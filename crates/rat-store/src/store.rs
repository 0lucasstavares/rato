use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;

use rusqlite::{params, Connection};
use tokio::sync::oneshot;

use rat_core::clock::Clock;
use rat_core::id::new_id;
use rat_proto::{Event, NewEvent, NewObservation, Observation, Project, WorkSession};

use crate::db::open_db;
use crate::error::StoreError;
use crate::rows::{
    decode_embedding, encode_embedding, Memory, MemoryFilter, NewMemory, NewPushback, Pushback,
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
    FtsObservations { query: String, limit: u32, reply: Reply<Vec<(String, f64)>> },
    FtsMemories { query: String, limit: u32, reply: Reply<Vec<(String, f64)>> },
    UnembeddedObservations { kinds: Vec<String>, limit: u32, reply: Reply<Vec<Observation>> },
    SetObservationEmbedding { obs_id: String, embedding: Vec<f32>, reply: Reply<()> },
    SetMemoryEmbedding { memory_id: String, embedding: Vec<f32>, reply: Reply<()> },
    AllObservationEmbeddings { limit: u32, reply: Reply<Vec<(String, Vec<f32>)>> },
    AllMemoryEmbeddings { limit: u32, reply: Reply<Vec<(String, Vec<f32>)>> },
    ObservationsByIds { ids: Vec<String>, reply: Reply<Vec<Observation>> },
    InsertPushback { pb: NewPushback, reply: Reply<Pushback> },
    RecentPushbacks { limit: u32, reply: Reply<Vec<Pushback>> },
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

    /// Full-text search over observations. Returns (obs_id, bm25_rank) pairs.
    pub async fn fts_observations(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<(String, f64)>, StoreError> {
        let (rtx, rrx) = oneshot::channel();
        self.tx
            .send(Cmd::FtsObservations { query, limit, reply: rtx })
            .map_err(|_| StoreError::ActorGone)?;
        rrx.await.map_err(|_| StoreError::ActorGone)?
    }

    /// Full-text search over memories. Returns (memory_id, bm25_rank) pairs.
    pub async fn fts_memories(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<(String, f64)>, StoreError> {
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

fn do_fts_observations(conn: &Connection, query: &str, limit: u32) -> Result<Vec<(String, f64)>, StoreError> {
    // Join FTS results back to observations to get the real id
    let mut stmt = conn.prepare(
        "SELECT o.id, bm25(observations_fts) AS rank
         FROM observations_fts f
         JOIN observations o ON o.rowid = f.rowid
         WHERE observations_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![query, limit], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn do_fts_memories(conn: &Connection, query: &str, limit: u32) -> Result<Vec<(String, f64)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT m.id, bm25(memories_fts) AS rank
         FROM memories_fts f
         JOIN memories m ON m.rowid = f.rowid
         WHERE memories_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![query, limit], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
    })?;
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
        assert_eq!(hits[0].0, obs.id);

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
        assert_eq!(mem_hits[0].0, mem.id);
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
}
