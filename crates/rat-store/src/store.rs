use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;

use rusqlite::{params, Connection};
use tokio::sync::oneshot;

use rat_core::clock::Clock;
use rat_core::id::new_id;
use rat_proto::{Event, NewEvent};

use crate::db::open_db;
use crate::error::StoreError;

enum Cmd {
    Append { ev: NewEvent, reply: oneshot::Sender<Result<Event, StoreError>> },
    Recent { limit: u32, reply: oneshot::Sender<Result<Vec<Event>, StoreError>> },
    Count { reply: oneshot::Sender<Result<u64, StoreError>> },
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
        }
    }
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
