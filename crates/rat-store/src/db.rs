use std::path::Path;

use rusqlite::Connection;

use crate::error::StoreError;

/// One entry per schema version; index i migrates user_version i → i+1.
/// Append only — never edit a shipped migration.
const MIGRATIONS: &[&str] = &[
    // v1: append-only event spine
    "CREATE TABLE events (
        id          TEXT PRIMARY KEY,
        ts          INTEGER NOT NULL,
        kind        TEXT NOT NULL,
        source      TEXT NOT NULL,
        project_id  TEXT,
        session_id  TEXT,
        payload     TEXT NOT NULL,
        lang        TEXT
    );
    CREATE INDEX idx_events_ts ON events(ts);
    CREATE INDEX idx_events_kind_ts ON events(kind, ts);",
    // v2: projects, work sessions, observations (M1 sensors)
    "CREATE TABLE projects (
        id          TEXT PRIMARY KEY,
        root_path   TEXT UNIQUE NOT NULL,
        name        TEXT NOT NULL,
        first_seen  INTEGER NOT NULL,
        last_seen   INTEGER NOT NULL
    );
    CREATE TABLE work_sessions (
        id            TEXT PRIMARY KEY,
        project_id    TEXT NOT NULL,
        started       INTEGER NOT NULL,
        last_activity INTEGER NOT NULL,
        ended         INTEGER,
        commands      INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_sessions_proj ON work_sessions(project_id, started);
    CREATE TABLE observations (
        id          TEXT PRIMARY KEY,
        event_id    TEXT,
        ts          INTEGER NOT NULL,
        kind        TEXT NOT NULL,
        project_id  TEXT,
        content     TEXT NOT NULL,
        meta        TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX idx_obs_ts ON observations(ts);
    CREATE INDEX idx_obs_kind_ts ON observations(kind, ts);",
];

pub fn open_db(path: &Path) -> Result<Connection, StoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut conn = Connection::open(path)?;
    // journal_mode returns a row, so query_row instead of pragma_update
    conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0))?;
    conn.execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
    migrate(&mut conn)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_schema_at_latest_version() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = open_db(&tmp.path().join("t.db")).unwrap();
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        conn.prepare("SELECT id, ts, kind, source, project_id, session_id, payload, lang FROM events")
            .unwrap();
        conn.prepare("SELECT id, root_path, name, first_seen, last_seen FROM projects").unwrap();
        conn.prepare("SELECT id, project_id, started, last_activity, ended, commands FROM work_sessions")
            .unwrap();
        conn.prepare("SELECT id, event_id, ts, kind, project_id, content, meta FROM observations")
            .unwrap();
    }

    #[test]
    fn reopen_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        drop(open_db(&path).unwrap());
        let conn = open_db(&path).unwrap();
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
    }

    #[test]
    fn v1_database_upgrades_to_v2_keeping_events() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        // hand-build a v1 database
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO events (id, ts, kind, source, payload) VALUES ('e1', 1, 'k', 's', 'null')",
                [],
            )
            .unwrap();
        }
        let conn = open_db(&path).unwrap();
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 2);
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn db_file_is_private_and_wal() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        let conn = open_db(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let jm: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(jm.to_lowercase(), "wal");
    }
}
