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
    fn open_creates_schema_at_version_1() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = open_db(&tmp.path().join("t.db")).unwrap();
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
        // events table exists
        conn.prepare("SELECT id, ts, kind, source, project_id, session_id, payload, lang FROM events")
            .unwrap();
    }

    #[test]
    fn reopen_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        drop(open_db(&path).unwrap());
        let conn = open_db(&path).unwrap();
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
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
