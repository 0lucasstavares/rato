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
}
