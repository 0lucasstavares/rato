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
    // v3: memories, pushbacks, disclosures, api_calls, FTS5, embedding BLOBs
    "ALTER TABLE work_sessions ADD COLUMN summary TEXT;
    CREATE TABLE memories (
        id               TEXT PRIMARY KEY,
        type             TEXT NOT NULL,
        project_id       TEXT,
        title            TEXT NOT NULL,
        body             TEXT NOT NULL,
        confidence       REAL NOT NULL DEFAULT 0.7,
        created          INTEGER NOT NULL,
        updated          INTEGER NOT NULL,
        source_event_ids TEXT NOT NULL DEFAULT '[]',
        archived         INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE pushbacks (
        id           TEXT PRIMARY KEY,
        ts           INTEGER NOT NULL,
        mode         TEXT NOT NULL,
        trigger      TEXT NOT NULL,
        severity     TEXT NOT NULL,
        title        TEXT NOT NULL,
        message_en   TEXT NOT NULL,
        message_pt   TEXT NOT NULL,
        evidence     TEXT NOT NULL,
        proposals    TEXT NOT NULL DEFAULT '[]',
        confidence   REAL NOT NULL,
        status       TEXT NOT NULL,
        decided_at   INTEGER,
        latency_ms   INTEGER
    );
    CREATE TABLE disclosures (
        id               TEXT PRIMARY KEY,
        ts               INTEGER NOT NULL,
        api_call_id      TEXT,
        model            TEXT NOT NULL,
        purpose          TEXT NOT NULL,
        memory_ids       TEXT NOT NULL DEFAULT '[]',
        observation_ids  TEXT NOT NULL DEFAULT '[]'
    );
    CREATE TABLE api_calls (
        id         TEXT PRIMARY KEY,
        ts         INTEGER NOT NULL,
        model      TEXT NOT NULL,
        purpose    TEXT NOT NULL,
        tokens_in  INTEGER,
        tokens_out INTEGER,
        cost_usd   REAL,
        ok         INTEGER NOT NULL,
        error      TEXT
    );
    CREATE TABLE metrics_daily (
        date       TEXT NOT NULL,
        project_id TEXT,
        metrics    TEXT NOT NULL,
        PRIMARY KEY (date, project_id)
    );
    CREATE VIRTUAL TABLE observations_fts USING fts5(content, content='', tokenize='unicode61');
    CREATE VIRTUAL TABLE memories_fts USING fts5(title, body, content='', tokenize='unicode61');
    CREATE TABLE vec_observations (obs_id TEXT PRIMARY KEY, embedding BLOB NOT NULL);
    CREATE TABLE vec_memories (memory_id TEXT PRIMARY KEY, embedding BLOB NOT NULL);
    CREATE INDEX idx_pushbacks_ts ON pushbacks(ts);
    CREATE INDEX idx_memories_project ON memories(project_id, updated);
    CREATE TRIGGER observations_fts_ai AFTER INSERT ON observations BEGIN
        INSERT INTO observations_fts(rowid, content) VALUES (new.rowid, new.content);
    END;
    CREATE TRIGGER observations_fts_ad AFTER DELETE ON observations BEGIN
        INSERT INTO observations_fts(observations_fts, rowid, content) VALUES ('delete', old.rowid, old.content);
    END;
    CREATE TRIGGER memories_fts_ai AFTER INSERT ON memories BEGIN
        INSERT INTO memories_fts(rowid, title, body) VALUES (new.rowid, new.title, new.body);
    END;
    CREATE TRIGGER memories_fts_ad AFTER DELETE ON memories BEGIN
        INSERT INTO memories_fts(memories_fts, rowid, title, body) VALUES ('delete', old.rowid, old.title, old.body);
    END;
    INSERT INTO observations_fts(rowid, content) SELECT rowid, content FROM observations;
    INSERT INTO memories_fts(rowid, title, body) SELECT rowid, title, body FROM memories;",
    // v4: approvals, actions, agent_runs, blobs (M4 Workbench)
    "CREATE TABLE approvals (
        id              TEXT PRIMARY KEY,
        created         INTEGER NOT NULL,
        kind            TEXT NOT NULL,
        risk            INTEGER NOT NULL,
        title           TEXT NOT NULL,
        reason          TEXT NOT NULL,
        cwd             TEXT,
        target          TEXT,
        agent_identity  TEXT NOT NULL,
        payload         TEXT NOT NULL,
        expected_impact TEXT NOT NULL,
        expires_at      INTEGER NOT NULL,
        status          TEXT NOT NULL DEFAULT 'pending',
        decided_at      INTEGER,
        decided_via     TEXT,
        decision_note   TEXT,
        execution       TEXT
    );
    CREATE INDEX idx_approvals_status_expires ON approvals(status, expires_at);
    CREATE TABLE actions (
        id          TEXT PRIMARY KEY,
        approval_id TEXT,
        kind        TEXT NOT NULL,
        payload     TEXT NOT NULL,
        started     INTEGER NOT NULL,
        ended       INTEGER,
        exit_code   INTEGER,
        output_blob TEXT
    );
    CREATE TABLE agent_runs (
        id             TEXT PRIMARY KEY,
        adapter        TEXT NOT NULL,
        task_title     TEXT NOT NULL,
        project_id     TEXT NOT NULL,
        worktree_path  TEXT NOT NULL,
        branch         TEXT NOT NULL,
        tmux_target    TEXT,
        mode           TEXT NOT NULL,
        status         TEXT NOT NULL,
        tokens         TEXT NOT NULL DEFAULT '{}',
        cost_usd       REAL NOT NULL DEFAULT 0.0,
        started        INTEGER NOT NULL,
        ended          INTEGER,
        result_summary TEXT,
        diffstat       TEXT
    );
    CREATE INDEX idx_agent_runs_status_started ON agent_runs(status, started);
    CREATE TABLE blobs (
        id      TEXT PRIMARY KEY,
        sha256  TEXT UNIQUE NOT NULL,
        bytes   BLOB NOT NULL,
        created INTEGER NOT NULL
    );",
    // v5: pins (M5 Eyes — screen/audio/clipboard capture records)
    "CREATE TABLE pins (
        id         TEXT PRIMARY KEY,
        kind       TEXT NOT NULL,
        media      TEXT NOT NULL,
        path       TEXT NOT NULL,
        created    INTEGER NOT NULL,
        expires_at INTEGER,
        reason     TEXT NOT NULL,
        meta       TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX idx_pins_expires ON pins(expires_at);",
    // v6: retention_status (M5 Eyes — last nightly prune time + counts, single row)
    "CREATE TABLE retention_status (
        id                   TEXT PRIMARY KEY,
        last_run_ms          INTEGER NOT NULL,
        observations_deleted INTEGER NOT NULL DEFAULT 0,
        pins_expired         INTEGER NOT NULL DEFAULT 0,
        api_calls_deleted    INTEGER NOT NULL DEFAULT 0
    );",
    // v7: voice utterances (M6 Voice — post-wake transcripts only)
    "CREATE TABLE voice_utterances (
        id        TEXT PRIMARY KEY,
        ts        INTEGER NOT NULL,
        lang      TEXT NOT NULL,
        text      TEXT NOT NULL,
        intent    TEXT,
        wake_word TEXT NOT NULL,
        handled   INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_voice_utterances_ts ON voice_utterances(ts);",
    // v8: terminals + dotfile edit audit trail (M7 Polish)
    "CREATE TABLE terminals (
        id           TEXT PRIMARY KEY,
        tty          TEXT NOT NULL,
        pid          INTEGER NOT NULL,
        emulator     TEXT NOT NULL,
        tmux_target  TEXT,
        role         TEXT NOT NULL,
        project_id   TEXT,
        cmd_hash     TEXT NOT NULL,
        first_seen   INTEGER NOT NULL,
        last_seen    INTEGER NOT NULL,
        meta         TEXT NOT NULL DEFAULT '{}',
        UNIQUE(tty, cmd_hash)
    );
    CREATE INDEX idx_terminals_tty ON terminals(tty);
    CREATE INDEX idx_terminals_role_last_seen ON terminals(role, last_seen);
    CREATE TABLE dotfile_edits (
        id          TEXT PRIMARY KEY,
        path        TEXT NOT NULL,
        kind        TEXT NOT NULL,
        before_blob TEXT NOT NULL,
        after_blob  TEXT NOT NULL,
        diff        TEXT NOT NULL,
        reason      TEXT NOT NULL,
        source      TEXT NOT NULL,
        risk        INTEGER NOT NULL,
        created     INTEGER NOT NULL,
        applied     INTEGER NOT NULL DEFAULT 1,
        reverted_by TEXT,
        meta        TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX idx_dotfile_edits_created ON dotfile_edits(created);",
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
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        conn.prepare(
            "SELECT id, ts, kind, source, project_id, session_id, payload, lang FROM events",
        )
        .unwrap();
        conn.prepare("SELECT id, root_path, name, first_seen, last_seen FROM projects")
            .unwrap();
        conn.prepare(
            "SELECT id, project_id, started, last_activity, ended, commands FROM work_sessions",
        )
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
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
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
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn v2_database_upgrades_to_v3_with_fts_backfill() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        // hand-build a v2 database with one observation row
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.execute_batch(MIGRATIONS[1]).unwrap();
            conn.pragma_update(None, "user_version", 2i64).unwrap();
            conn.execute(
                "INSERT INTO observations (id, ts, kind, project_id, content, meta)
                 VALUES ('obs1', 1000, 'shell_cmd', NULL, 'cargo build --release', '{}')",
                [],
            )
            .unwrap();
        }
        let conn = open_db(&path).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        // FTS backfill: should find the observation via FTS match
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM observations_fts WHERE observations_fts MATCH 'cargo'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "FTS backfill should find existing observations");
        // summary column was added
        conn.prepare("SELECT summary FROM work_sessions").unwrap();
    }

    #[test]
    fn observations_fts_delete_path_removes_from_index() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = open_db(&tmp.path().join("t.db")).unwrap();

        // Insert a row directly — the AFTER INSERT trigger populates the FTS index.
        conn.execute(
            "INSERT INTO observations (id, ts, kind, project_id, content, meta)
             VALUES ('obs_del_test', 1000, 'shell_cmd', NULL, 'deleteme unique token xyzzy', '{}')",
            [],
        )
        .unwrap();

        // Confirm FTS match exists.
        let before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM observations_fts WHERE observations_fts MATCH 'xyzzy'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, 1, "FTS index should contain the row after insert");

        // Delete the observation row — the AFTER DELETE trigger should clean up the FTS index.
        conn.execute("DELETE FROM observations WHERE id = 'obs_del_test'", [])
            .unwrap();

        // Confirm FTS match is gone.
        let after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM observations_fts WHERE observations_fts MATCH 'xyzzy'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            after, 0,
            "FTS index should be empty after deleting the observation"
        );
    }

    #[test]
    fn v3_database_upgrades_to_v4_keeping_data() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        // hand-build a v3 database with rows in v3 tables
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.execute_batch(MIGRATIONS[1]).unwrap();
            conn.execute_batch(MIGRATIONS[2]).unwrap();
            conn.pragma_update(None, "user_version", 3i64).unwrap();
            conn.execute(
                "INSERT INTO memories (id, type, project_id, title, body, confidence, created, updated, source_event_ids, archived)
                 VALUES ('m1', 'note', NULL, 'Test memory', 'Body text', 0.9, 1000, 1000, '[]', 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO pushbacks (id, ts, mode, trigger, severity, title, message_en, message_pt, evidence, proposals, confidence, status)
                 VALUES ('pb1', 1000, 'mentor', 'stuck_loop', 'warn', 'Title', 'Msg EN', 'Msg PT', '[]', '[]', 0.8, 'shown')",
                [],
            )
            .unwrap();
        }
        // open_db should run v4 (and v5) migrations and reach the latest version
        let conn = open_db(&path).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        // v3 data is still there
        let mem_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mem_count, 1, "existing memory row must survive migration");
        let pb_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM pushbacks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pb_count, 1, "existing pushback row must survive migration");
        // v4 tables exist
        conn.prepare("SELECT id, created, kind, risk, title, reason, cwd, target, agent_identity, payload, expected_impact, expires_at, status, decided_at, decided_via, decision_note, execution FROM approvals").unwrap();
        conn.prepare("SELECT id, approval_id, kind, payload, started, ended, exit_code, output_blob FROM actions").unwrap();
        conn.prepare("SELECT id, adapter, task_title, project_id, worktree_path, branch, tmux_target, mode, status, tokens, cost_usd, started, ended, result_summary, diffstat FROM agent_runs").unwrap();
        conn.prepare("SELECT id, sha256, bytes, created FROM blobs")
            .unwrap();
        // v4 indexes exist (query via sqlite_master)
        let idx: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_approvals_status_expires'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(idx, 1, "idx_approvals_status_expires must exist");
        let idx2: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_agent_runs_status_started'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(idx2, 1, "idx_agent_runs_status_started must exist");
    }

    #[test]
    fn db_file_is_private_and_wal() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        let conn = open_db(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let jm: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(jm.to_lowercase(), "wal");
    }

    #[test]
    fn v4_database_upgrades_to_v5_keeping_data() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        // hand-build a v4 database with rows in v4 tables
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.execute_batch(MIGRATIONS[1]).unwrap();
            conn.execute_batch(MIGRATIONS[2]).unwrap();
            conn.execute_batch(MIGRATIONS[3]).unwrap();
            conn.pragma_update(None, "user_version", 4i64).unwrap();
            conn.execute(
                "INSERT INTO approvals (id, created, kind, risk, title, reason, agent_identity,
                                        payload, expected_impact, expires_at, status)
                 VALUES ('a1', 1000, 'shell', 1, 'Test approval', 'Because', 'claude',
                         '{}', '{}', 9999999, 'pending')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO agent_runs (id, adapter, task_title, project_id, worktree_path,
                                         branch, mode, status, started)
                 VALUES ('ar1', 'claude', 'Test run', 'proj1', '/tmp', 'main', 'headless',
                         'running', 1000)",
                [],
            )
            .unwrap();
        }
        // open_db should run v5 migration and reach version 5
        let conn = open_db(&path).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        // v4 data is still there
        let ap_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM approvals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ap_count, 1, "existing approval row must survive migration");
        let ar_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ar_count, 1, "existing agent_run row must survive migration");
        // v5 table exists with expected columns
        conn.prepare("SELECT id, kind, media, path, created, expires_at, reason, meta FROM pins")
            .unwrap();
        // v5 index exists
        let idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_pins_expires'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1, "idx_pins_expires must exist");
    }

    #[test]
    fn v5_database_upgrades_to_v6_keeping_data() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        // hand-build a v5 database with a pin row
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[0..5] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 5i64).unwrap();
            conn.execute(
                "INSERT INTO pins (id, kind, media, path, created, expires_at, reason, meta)
                 VALUES ('p1', 'manual', 'screen', '/tmp/p1', 1000, NULL, 'because', '{}')",
                [],
            )
            .unwrap();
        }
        // open_db should run the v6 migration and reach the latest version
        let conn = open_db(&path).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        // v5 data is still there
        let pin_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM pins", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pin_count, 1, "existing pin row must survive migration");
        // v6 table exists with expected columns
        conn.prepare(
            "SELECT id, last_run_ms, observations_deleted, pins_expired, api_calls_deleted FROM retention_status",
        )
        .unwrap();
    }

    #[test]
    fn v6_database_upgrades_to_v7_keeping_retention_status() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[0..6] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 6i64).unwrap();
            conn.execute(
                "INSERT INTO retention_status
                 (id, last_run_ms, observations_deleted, pins_expired, api_calls_deleted)
                 VALUES ('last', 1000, 1, 2, 3)",
                [],
            )
            .unwrap();
        }

        let conn = open_db(&path).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        let last_run: i64 = conn
            .query_row(
                "SELECT last_run_ms FROM retention_status WHERE id = 'last'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last_run, 1000);
        conn.prepare("SELECT id, ts, lang, text, intent, wake_word, handled FROM voice_utterances")
            .unwrap();
    }

    #[test]
    fn v7_database_upgrades_to_v8_keeping_voice_utterances() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[0..7] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 7i64).unwrap();
            conn.execute(
                "INSERT INTO voice_utterances
                 (id, ts, lang, text, intent, wake_word, handled)
                 VALUES ('u1', 1000, 'en', 'hello', 'chat', 'hey rat', 1)",
                [],
            )
            .unwrap();
        }

        let conn = open_db(&path).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        let text: String = conn
            .query_row(
                "SELECT text FROM voice_utterances WHERE id = 'u1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(text, "hello");
        conn.prepare(
            "SELECT id, tty, pid, emulator, tmux_target, role, project_id, cmd_hash, first_seen, last_seen, meta FROM terminals",
        )
        .unwrap();
        conn.prepare(
            "SELECT id, path, kind, before_blob, after_blob, diff, reason, source, risk, created, applied, reverted_by, meta FROM dotfile_edits",
        )
        .unwrap();
    }
}
