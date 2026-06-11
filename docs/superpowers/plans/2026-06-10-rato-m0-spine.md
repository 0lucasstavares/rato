# RATO M0 — Spine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the M0 "spine" of RATO (per `docs/ARCHITECTURE.md` §18): a Rust workspace with the `ratd` daemon (Unix-socket NDJSON-RPC, SQLite event store) and the `rat` CLI (status/emit/events/install/doctor), installable as a `systemd --user` service.

**Architecture:** Cargo workspace of 5 crates. `rat-core` (clock/ids/paths), `rat-proto` (RPC + event types), `rat-store` (SQLite behind a single-writer actor thread), `rat-daemon` (lib: paths-free server + bin: `ratd`), `rat-cli` (bin: `rat`). The daemon owns all state; the CLI is a thin NDJSON-RPC client. M0 acceptance: `rat status` round-trips, events persist across daemon restarts, `rat install` produces a working systemd user unit.

**Tech Stack:** Rust 2021 (toolchain 1.96 via rustup at `~/.cargo/bin`), tokio, clap v4, rusqlite (bundled), ulid, serde/serde_json, tracing, anyhow/thiserror, libc; tests with tokio::test, tempfile, assert_cmd, predicates.

**Environment note for every shell command in this plan:** cargo is NOT on the default PATH. Prefix sessions with:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

All paths below are relative to the repo root `/home/lucas-tavares/rato`.

---

## File structure (locked in)

```
Cargo.toml                          # workspace
.gitignore
rustfmt.toml
justfile
README.md
docs/ARCHITECTURE.md                # copied from ~/RATO-ARCHITECTURE.md
docs/superpowers/plans/...          # this plan
crates/rat-core/src/lib.rs          # modules: clock, id, paths
crates/rat-core/src/clock.rs        # Clock trait, SystemClock, FakeClock
crates/rat-core/src/id.rs           # new_id() → ULID string
crates/rat-core/src/paths.rs        # runtime/data dirs, socket/db paths, 0700 dirs
crates/rat-proto/src/lib.rs         # PROTO_VERSION, Request/Response, Hello/Status/Event types
crates/rat-store/src/lib.rs         # modules: db, error, store
crates/rat-store/src/db.rs          # open_db + hand-rolled user_version migrations
crates/rat-store/src/error.rs       # StoreError
crates/rat-store/src/store.rs       # Store handle + actor thread (append/recent/count)
crates/rat-daemon/src/lib.rs        # module: server
crates/rat-daemon/src/server.rs     # ServerCtx, serve(), dispatch()
crates/rat-daemon/src/main.rs       # ratd binary
crates/rat-daemon/tests/rpc.rs      # integration test over a temp UDS
crates/rat-cli/src/main.rs          # rat binary (clap)
crates/rat-cli/src/client.rs        # NDJSON-RPC client (hello handshake)
crates/rat-cli/src/install.rs       # systemd unit writer
crates/rat-cli/src/doctor.rs        # environment checks
crates/rat-cli/tests/cli.rs         # assert_cmd tests against in-process daemon
packaging/shell/rat-init.sh         # shell alias snippet (documented, not auto-installed)
```

---

### Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `rustfmt.toml`, `README.md`, `justfile`, `docs/ARCHITECTURE.md`
- Create: empty crate skeletons for all 5 crates

- [ ] **Step 1: Create workspace root files**

`Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/rat-core",
    "crates/rat-proto",
    "crates/rat-store",
    "crates/rat-daemon",
    "crates/rat-cli",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4", features = ["derive"] }
rusqlite = { version = "0.32", features = ["bundled"] }
ulid = "1"
libc = "0.2"
tempfile = "3"
assert_cmd = "2"
predicates = "3"
```

`.gitignore`:

```
/target
```

`rustfmt.toml`:

```toml
edition = "2021"
```

`README.md`:

```markdown
# RATO

Single-user, local-first Linux developer companion. See `docs/ARCHITECTURE.md`.

## Build

    export PATH="$HOME/.cargo/bin:$PATH"
    cargo build --workspace

## Install (systemd --user)

    cargo build --release --workspace
    ./target/release/rat install
    systemctl --user status ratd

## Dev

    cargo test --workspace
```

`justfile`:

```just
export PATH := env_var("HOME") + "/.cargo/bin:" + env_var("PATH")

default: test

build:
    cargo build --workspace

test:
    cargo test --workspace

release:
    cargo build --release --workspace

install: release
    ./target/release/rat install
```

- [ ] **Step 2: Copy the architecture spec into the repo**

Run: `cp /home/lucas-tavares/RATO-ARCHITECTURE.md /home/lucas-tavares/rato/docs/ARCHITECTURE.md`

- [ ] **Step 3: Create the 5 crate skeletons**

For each crate create `crates/<name>/Cargo.toml` and `crates/<name>/src/lib.rs` (or `main.rs`):

`crates/rat-core/Cargo.toml`:

```toml
[package]
name = "rat-core"
version.workspace = true
edition.workspace = true

[dependencies]
ulid = { workspace = true }
libc = { workspace = true }
```

`crates/rat-core/src/lib.rs`: `// modules added in Task 2` (empty file is fine)

`crates/rat-proto/Cargo.toml`:

```toml
[package]
name = "rat-proto"
version.workspace = true
edition.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
```

`crates/rat-proto/src/lib.rs`: empty.

`crates/rat-store/Cargo.toml`:

```toml
[package]
name = "rat-store"
version.workspace = true
edition.workspace = true

[dependencies]
rat-core = { path = "../rat-core" }
rat-proto = { path = "../rat-proto" }
rusqlite = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

`crates/rat-store/src/lib.rs`: empty.

`crates/rat-daemon/Cargo.toml`:

```toml
[package]
name = "rat-daemon"
version.workspace = true
edition.workspace = true

[lib]
name = "rat_daemon"

[[bin]]
name = "ratd"
path = "src/main.rs"

[dependencies]
rat-core = { path = "../rat-core" }
rat-proto = { path = "../rat-proto" }
rat-store = { path = "../rat-store" }
anyhow = { workspace = true }
clap = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

`crates/rat-daemon/src/lib.rs`: empty. `crates/rat-daemon/src/main.rs`: `fn main() {}` (replaced in Task 7).

`crates/rat-cli/Cargo.toml`:

```toml
[package]
name = "rat-cli"
version.workspace = true
edition.workspace = true

[[bin]]
name = "rat"
path = "src/main.rs"

[dependencies]
rat-core = { path = "../rat-core" }
rat-proto = { path = "../rat-proto" }
anyhow = { workspace = true }
clap = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }

[dev-dependencies]
rat-daemon = { path = "../rat-daemon" }
rat-store = { path = "../rat-store" }
assert_cmd = { workspace = true }
predicates = { workspace = true }
tempfile = { workspace = true }
```

`crates/rat-cli/src/main.rs`: `fn main() {}` (replaced in Task 8).

- [ ] **Step 4: Verify the workspace compiles**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd /home/lucas-tavares/rato && cargo check --workspace`
Expected: success (warnings about empty crates are fine). First run downloads deps and builds bundled SQLite — allow several minutes.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "chore: scaffold rato workspace (5 crates, docs, justfile)"
```

---

### Task 2: rat-core — Clock, ids, paths

**Files:**
- Create: `crates/rat-core/src/clock.rs`, `crates/rat-core/src/id.rs`, `crates/rat-core/src/paths.rs`
- Modify: `crates/rat-core/src/lib.rs`

- [ ] **Step 1: Write the failing tests (inside the new modules)**

`crates/rat-core/src/lib.rs`:

```rust
pub mod clock;
pub mod id;
pub mod paths;
```

`crates/rat-core/src/clock.rs` (tests included at the bottom — implementation in step 3; for a strict TDD fail, create the file with only the trait + tests first):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_advances() {
        let c = FakeClock::at(1_000);
        assert_eq!(c.now_ms(), 1_000);
        c.advance(500);
        assert_eq!(c.now_ms(), 1_500);
    }

    #[test]
    fn system_clock_is_recent() {
        // any moment after 2024-01-01 in ms
        assert!(SystemClock.now_ms() > 1_704_067_200_000);
    }
}
```

`crates/rat-core/src/id.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_26_chars_and_unique() {
        let a = new_id();
        let b = new_id();
        assert_eq!(a.len(), 26);
        assert_ne!(a, b);
    }
}
```

`crates/rat-core/src/paths.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_lives_under_runtime_dir() {
        assert!(socket_path().ends_with("rato/ratd.sock") || socket_path().to_string_lossy().contains("/tmp/rato-"));
    }

    #[test]
    fn db_lives_under_data_dir() {
        assert!(db_path().ends_with("rato/rato.db"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `cargo test -p rat-core`
Expected: compile errors (`FakeClock` etc. not defined).

- [ ] **Step 3: Implement**

`crates/rat-core/src/clock.rs` (above the tests):

```rust
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source of "now" in milliseconds since the Unix epoch. All time in RATO
/// flows through this trait so tests can use a FakeClock.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_millis() as i64
    }
}

#[derive(Debug, Default)]
pub struct FakeClock {
    now: AtomicI64,
}

impl FakeClock {
    pub fn at(ms: i64) -> Arc<Self> {
        Arc::new(Self { now: AtomicI64::new(ms) })
    }

    pub fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}
```

`crates/rat-core/src/id.rs`:

```rust
/// New ULID as a 26-char Crockford base32 string. Sortable by creation time.
pub fn new_id() -> String {
    ulid::Ulid::new().to_string()
}
```

`crates/rat-core/src/paths.rs`:

```rust
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// $XDG_RUNTIME_DIR/rato, falling back to /tmp/rato-<uid>.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => PathBuf::from(d).join("rato"),
        None => PathBuf::from(format!("/tmp/rato-{}", unsafe { libc::getuid() })),
    }
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("ratd.sock")
}

/// $XDG_DATA_HOME/rato, falling back to ~/.local/share/rato.
pub fn data_dir() -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(d) => PathBuf::from(d).join("rato"),
        None => PathBuf::from(std::env::var_os("HOME").expect("HOME not set"))
            .join(".local/share/rato"),
    }
}

pub fn db_path() -> PathBuf {
    data_dir().join("rato.db")
}

/// Create a directory (and parents) and clamp it to 0700.
pub fn ensure_private_dir(p: &Path) -> io::Result<()> {
    std::fs::create_dir_all(p)?;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p rat-core`
Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/rat-core && git commit -m "feat(core): clock abstraction, ULID ids, XDG paths"
```

---

### Task 3: rat-proto — RPC and event types

**Files:**
- Create: `crates/rat-proto/src/lib.rs` (full contents)

- [ ] **Step 1: Write the failing serde round-trip tests + types**

`crates/rat-proto/src/lib.rs`:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bump on any wire-incompatible change. Checked in the `hello` handshake.
pub const PROTO_VERSION: u32 = 1;

pub mod methods {
    pub const HELLO: &str = "hello";
    pub const STATUS: &str = "status";
    pub const EVENTS_APPEND: &str = "events.append";
    pub const EVENTS_RECENT: &str = "events.recent";
}

pub mod errcodes {
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INTERNAL: i64 = -32000;
    pub const HELLO_REQUIRED: i64 = -32001;
    pub const PROTO_MISMATCH: i64 = -32002;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self { id, result: Some(result), error: None }
    }

    pub fn err(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self { id, result: None, error: Some(RpcError { code, message: message.into() }) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HelloParams {
    pub proto_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HelloResult {
    pub proto_version: u32,
    pub server_version: String,
}

/// Client-supplied half of an event; the store assigns `id` and `ts`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NewEvent {
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub ts: i64,
    pub kind: String,
    pub source: String,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub payload: Value,
    pub lang: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusResult {
    pub version: String,
    pub proto_version: u32,
    pub uptime_ms: i64,
    pub event_count: u64,
    pub db_path: String,
}

fn default_limit() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentParams {
    #[serde(default = "default_limit")]
    pub limit: u32,
}

impl Default for RecentParams {
    fn default() -> Self {
        Self { limit: default_limit() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_round_trips_and_defaults_params() {
        let r: Request = serde_json::from_str(r#"{"id":1,"method":"status"}"#).unwrap();
        assert_eq!(r.params, Value::Null);
        let s = serde_json::to_string(&r).unwrap();
        let r2: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn response_ok_omits_error_field() {
        let s = serde_json::to_string(&Response::ok(7, json!({"a":1}))).unwrap();
        assert!(!s.contains("error"));
        assert!(s.contains("\"id\":7"));
    }

    #[test]
    fn response_err_omits_result_field() {
        let s = serde_json::to_string(&Response::err(7, errcodes::HELLO_REQUIRED, "hello required")).unwrap();
        assert!(!s.contains("result"));
        assert!(s.contains("-32001"));
    }

    #[test]
    fn new_event_minimal_json_parses() {
        let e: NewEvent = serde_json::from_str(r#"{"kind":"k","source":"s"}"#).unwrap();
        assert_eq!(e.payload, Value::Null);
        assert_eq!(e.project_id, None);
    }

    #[test]
    fn recent_params_default_limit_is_50() {
        let p: RecentParams = serde_json::from_str("{}").unwrap();
        assert_eq!(p.limit, 50);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p rat-proto`
Expected: PASS (types and tests land together in this task; the compile itself is the failure gate).

- [ ] **Step 3: Commit**

```bash
git add crates/rat-proto && git commit -m "feat(proto): NDJSON-RPC v1 types, hello/status/events methods"
```

---

### Task 4: rat-store — open + migrations

**Files:**
- Create: `crates/rat-store/src/db.rs`, `crates/rat-store/src/error.rs`
- Modify: `crates/rat-store/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

`crates/rat-store/src/lib.rs`:

```rust
pub mod db;
pub mod error;
pub mod store;
```

(`store.rs` arrives in Task 5 — create it now as an empty file so the crate compiles: `touch crates/rat-store/src/store.rs`.)

`crates/rat-store/src/db.rs` — tests at the bottom of the file:

```rust
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
        conn.prepare("SELECT id, ts, kind, source, project_id, session_id, payload, lang FROM events").unwrap();
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p rat-store`
Expected: compile error (`open_db` not defined).

- [ ] **Step 3: Implement**

`crates/rat-store/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("store thread is gone")]
    ActorGone,
}
```

`crates/rat-store/src/db.rs` (above the tests):

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p rat-store`
Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/rat-store && git commit -m "feat(store): sqlite open with WAL, 0600 perms, user_version migrations"
```

---

### Task 5: rat-store — event store actor

**Files:**
- Create: `crates/rat-store/src/store.rs` (replace the empty file)

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/rat-store/src/store.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `cargo test -p rat-store`
Expected: compile errors (`Store` not defined).

- [ ] **Step 3: Implement the actor (above the tests)**

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p rat-store`
Expected: all rat-store tests PASS (db + store).

- [ ] **Step 5: Commit**

```bash
git add crates/rat-store && git commit -m "feat(store): single-writer event store actor (append/recent/count)"
```

---

### Task 6: rat-daemon lib — RPC server + integration test

**Files:**
- Create: `crates/rat-daemon/src/server.rs`, `crates/rat-daemon/tests/rpc.rs`
- Modify: `crates/rat-daemon/src/lib.rs`

- [ ] **Step 1: Write the failing integration test**

`crates/rat-daemon/src/lib.rs`:

```rust
pub mod server;
```

(`touch crates/rat-daemon/src/server.rs` so it compiles enough to show the real test failure.)

`crates/rat-daemon/tests/rpc.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rat_core::clock::SystemClock;
use rat_daemon::server::{serve, ServerCtx};
use rat_proto::{errcodes, Response, PROTO_VERSION};
use rat_store::store::Store;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

async fn start() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("ratd.sock");
    let db = tmp.path().join("rato.db");
    let store = Store::open(&db, Arc::new(SystemClock)).unwrap();
    let ctx = Arc::new(ServerCtx { store, started: Instant::now(), db_path: db });
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(serve(listener, ctx));
    (tmp, socket)
}

struct TestClient {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    w: tokio::net::unix::OwnedWriteHalf,
}

impl TestClient {
    async fn connect(socket: &Path) -> Self {
        let s = UnixStream::connect(socket).await.unwrap();
        let (r, w) = s.into_split();
        Self { lines: BufReader::new(r).lines(), w }
    }

    async fn send(&mut self, req: serde_json::Value) -> Response {
        let mut buf = serde_json::to_vec(&req).unwrap();
        buf.push(b'\n');
        self.w.write_all(&buf).await.unwrap();
        let line = self.lines.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

#[tokio::test]
async fn methods_before_hello_are_rejected() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    let resp = c.send(json!({"id": 1, "method": "status"})).await;
    assert_eq!(resp.error.unwrap().code, errcodes::HELLO_REQUIRED);
}

#[tokio::test]
async fn hello_with_wrong_proto_version_is_rejected() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    let resp = c
        .send(json!({"id": 1, "method": "hello", "params": {"proto_version": 999}}))
        .await;
    assert_eq!(resp.error.unwrap().code, errcodes::PROTO_MISMATCH);
    // and it did NOT unlock the connection
    let resp = c.send(json!({"id": 2, "method": "status"})).await;
    assert_eq!(resp.error.unwrap().code, errcodes::HELLO_REQUIRED);
}

#[tokio::test]
async fn full_round_trip_hello_status_append_recent() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;

    let hello = c
        .send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}}))
        .await;
    assert!(hello.error.is_none());

    let status = c.send(json!({"id": 2, "method": "status"})).await;
    let status = status.result.unwrap();
    assert_eq!(status["event_count"], 0);

    let appended = c
        .send(json!({"id": 3, "method": "events.append",
                     "params": {"kind": "test_event", "source": "test", "payload": {"n": 1}}}))
        .await;
    let appended = appended.result.unwrap();
    assert_eq!(appended["kind"], "test_event");

    let recent = c.send(json!({"id": 4, "method": "events.recent", "params": {"limit": 10}})).await;
    let recent = recent.result.unwrap();
    assert_eq!(recent.as_array().unwrap().len(), 1);
    assert_eq!(recent[0]["payload"]["n"], 1);
}

#[tokio::test]
async fn invalid_json_returns_invalid_request() {
    let (_tmp, socket) = start().await;
    let s = UnixStream::connect(&socket).await.unwrap();
    let (r, mut w) = s.into_split();
    w.write_all(b"this is not json\n").await.unwrap();
    let mut lines = BufReader::new(r).lines();
    let line = lines.next_line().await.unwrap().unwrap();
    let resp: Response = serde_json::from_str(&line).unwrap();
    assert_eq!(resp.error.unwrap().code, errcodes::INVALID_REQUEST);
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let (_tmp, socket) = start().await;
    let mut c = TestClient::connect(&socket).await;
    c.send(json!({"id": 1, "method": "hello", "params": {"proto_version": PROTO_VERSION}})).await;
    let resp = c.send(json!({"id": 2, "method": "nope"})).await;
    assert_eq!(resp.error.unwrap().code, errcodes::METHOD_NOT_FOUND);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p rat-daemon`
Expected: compile errors (`serve`, `ServerCtx` not defined).

- [ ] **Step 3: Implement the server**

`crates/rat-daemon/src/server.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use rat_proto::{
    errcodes, methods, HelloParams, HelloResult, NewEvent, RecentParams, Request, Response,
    StatusResult, PROTO_VERSION,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use rat_store::store::Store;

pub struct ServerCtx {
    pub store: Store,
    pub started: Instant,
    pub db_path: PathBuf,
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
        Err(e) => return Response::err(0, errcodes::INVALID_REQUEST, format!("invalid request: {e}")),
    };
    match req.method.as_str() {
        methods::HELLO => {
            let params: HelloParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad hello params: {e}")),
            };
            if params.proto_version != PROTO_VERSION {
                return Response::err(
                    req.id,
                    errcodes::PROTO_MISMATCH,
                    format!("daemon speaks proto v{PROTO_VERSION}, client sent v{}", params.proto_version),
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
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad event: {e}")),
            };
            if ev.kind.is_empty() || ev.source.is_empty() {
                return Response::err(req.id, errcodes::INVALID_REQUEST, "kind and source are required");
            }
            match ctx.store.append(ev).await {
                Ok(event) => Response::ok(req.id, serde_json::to_value(event).expect("serializes")),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        methods::EVENTS_RECENT => {
            let params: RecentParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad params: {e}")),
            };
            match ctx.store.recent(params.limit.min(1000)).await {
                Ok(events) => Response::ok(req.id, serde_json::to_value(events).expect("serializes")),
                Err(e) => Response::err(req.id, errcodes::INTERNAL, e.to_string()),
            }
        }
        other => Response::err(req.id, errcodes::METHOD_NOT_FOUND, format!("unknown method: {other}")),
    }
}
```

Note: `RecentParams` with `params: null` — `serde_json::from_value(Value::Null)` into a struct with all-default fields fails on Null. The test always sends `params`, but the CLI may not. Guard in dispatch: replace the `EVENTS_RECENT` params parse with:

```rust
let params: RecentParams = if req.params.is_null() {
    RecentParams::default()
} else {
    match serde_json::from_value(req.params) {
        Ok(p) => p,
        Err(e) => return Response::err(req.id, errcodes::INVALID_REQUEST, format!("bad params: {e}")),
    }
};
```

Apply the same null-guard shape to `STATUS` (it takes no params, so nothing to parse — already fine).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p rat-daemon`
Expected: 5 integration tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/rat-daemon && git commit -m "feat(daemon): UDS NDJSON-RPC server with hello gate and event methods"
```

---

### Task 7: ratd binary

**Files:**
- Modify: `crates/rat-daemon/src/main.rs` (replace stub)

- [ ] **Step 1: Implement main**

`crates/rat-daemon/src/main.rs`:

```rust
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use rat_core::clock::SystemClock;
use rat_core::paths;
use rat_daemon::server::{serve, ServerCtx};
use rat_proto::NewEvent;
use rat_store::store::Store;

/// RATO daemon: observes, remembers, critiques. M0: event spine + RPC.
#[derive(Parser)]
#[command(name = "ratd", version)]
struct Args {
    /// Socket path (default: $XDG_RUNTIME_DIR/rato/ratd.sock)
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Database path (default: ~/.local/share/rato/rato.db)
    #[arg(long)]
    db: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_env("RAT_LOG").unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let args = Args::parse();
    let socket = args.socket.unwrap_or_else(paths::socket_path);
    let db = args.db.unwrap_or_else(paths::db_path);

    if let Some(dir) = socket.parent() {
        paths::ensure_private_dir(dir).context("creating runtime dir")?;
    }
    if let Some(dir) = db.parent() {
        paths::ensure_private_dir(dir).context("creating data dir")?;
    }

    // Stale-socket handling: if something answers, another ratd is running.
    if socket.exists() {
        match tokio::net::UnixStream::connect(&socket).await {
            Ok(_) => anyhow::bail!("ratd already running on {}", socket.display()),
            Err(_) => {
                tracing::info!("removing stale socket {}", socket.display());
                std::fs::remove_file(&socket)?;
            }
        }
    }

    let store = Store::open(&db, Arc::new(SystemClock)).context("opening event store")?;
    let listener = tokio::net::UnixListener::bind(&socket)
        .with_context(|| format!("binding {}", socket.display()))?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;

    store
        .append(NewEvent { kind: "daemon_started".into(), source: "ratd".into(), ..Default::default() })
        .await?;

    tracing::info!("ratd {} listening on {}", env!("CARGO_PKG_VERSION"), socket.display());
    tracing::info!("event store at {}", db.display());

    let ctx = Arc::new(ServerCtx { store, started: Instant::now(), db_path: db });

    tokio::select! {
        _ = serve(listener, ctx) => {}
        _ = shutdown_signal() => {
            tracing::info!("shutting down");
        }
    }

    let _ = std::fs::remove_file(&socket);
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
```

- [ ] **Step 2: Build and smoke-test manually**

```bash
cargo build -p rat-daemon
tmpd=$(mktemp -d)
./target/debug/ratd --socket "$tmpd/s.sock" --db "$tmpd/d.db" &
sleep 1
printf '{"id":1,"method":"hello","params":{"proto_version":1}}\n{"id":2,"method":"status"}\n' | timeout 2 nc -U "$tmpd/s.sock" -q1 || true
kill %1
```

Expected: two JSON lines — a hello result with `server_version`, then a status with `"event_count":1` (the `daemon_started` event). Socket file removed after kill (SIGTERM).

(If `nc` lacks `-U`, verify instead with `rat status` after Task 8.)

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --workspace`
Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/rat-daemon && git commit -m "feat(daemon): ratd binary — args, tracing, stale-socket guard, graceful shutdown"
```

---

### Task 8: rat CLI — client, status, emit, events recent

**Files:**
- Create: `crates/rat-cli/src/client.rs`, `crates/rat-cli/tests/cli.rs`
- Modify: `crates/rat-cli/src/main.rs` (replace stub)

- [ ] **Step 1: Write the failing integration test**

`crates/rat-cli/tests/cli.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use predicates::str::contains;

/// Run an in-process daemon on a background thread; return the socket path.
fn start_daemon(tmp: &Path) -> PathBuf {
    let socket = tmp.join("ratd.sock");
    let db = tmp.join("rato.db");
    let socket2 = socket.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let store = rat_store::store::Store::open(&db, Arc::new(rat_core::clock::SystemClock)).unwrap();
            let ctx = Arc::new(rat_daemon::server::ServerCtx {
                store,
                started: Instant::now(),
                db_path: db,
            });
            let listener = tokio::net::UnixListener::bind(&socket2).unwrap();
            rat_daemon::server::serve(listener, ctx).await;
        });
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(20));
    }
    socket
}

#[test]
fn status_emit_and_recent_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = start_daemon(tmp.path());
    let sock = socket.to_str().unwrap();

    Command::cargo_bin("rat").unwrap()
        .args(["--socket", sock, "status"])
        .assert()
        .success()
        .stdout(contains("ratd").and(contains("events:")));

    Command::cargo_bin("rat").unwrap()
        .args(["--socket", sock, "emit", "test_event", "--payload", r#"{"n":1}"#])
        .assert()
        .success();

    Command::cargo_bin("rat").unwrap()
        .args(["--socket", sock, "events", "recent"])
        .assert()
        .success()
        .stdout(contains("test_event"));
}

#[test]
fn status_fails_cleanly_when_daemon_is_down() {
    let tmp = tempfile::tempdir().unwrap();
    let sock = tmp.path().join("nope.sock");

    Command::cargo_bin("rat").unwrap()
        .args(["--socket", sock.to_str().unwrap(), "status"])
        .assert()
        .failure()
        .stderr(contains("connecting"));
}
```

Add `use predicates::prelude::PredicateBooleanExt;` at the top if `.and(...)` doesn't resolve.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p rat-cli`
Expected: compile/test failure (stub `main` has no subcommands).

- [ ] **Step 3: Implement client and main**

`crates/rat-cli/src/client.rs`:

```rust
use std::path::Path;

use anyhow::Context;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

use rat_proto::{methods, HelloParams, HelloResult, Request, Response, StatusResult, PROTO_VERSION};

pub struct Client {
    lines: Lines<BufReader<OwnedReadHalf>>,
    w: OwnedWriteHalf,
    next_id: u64,
}

impl Client {
    pub async fn connect(socket: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket)
            .await
            .with_context(|| format!("connecting to {} (is ratd running?)", socket.display()))?;
        let (r, w) = stream.into_split();
        let mut client = Self { lines: BufReader::new(r).lines(), w, next_id: 0 };
        let hello: HelloResult = serde_json::from_value(
            client
                .call(methods::HELLO, serde_json::to_value(HelloParams { proto_version: PROTO_VERSION })?)
                .await?,
        )?;
        anyhow::ensure!(
            hello.proto_version == PROTO_VERSION,
            "protocol mismatch: daemon v{}, rat v{}",
            hello.proto_version,
            PROTO_VERSION
        );
        Ok(client)
    }

    pub async fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.next_id += 1;
        let req = Request { id: self.next_id, method: method.to_string(), params };
        let mut buf = serde_json::to_vec(&req)?;
        buf.push(b'\n');
        self.w.write_all(&buf).await?;
        let line = self.lines.next_line().await?.context("daemon closed the connection")?;
        let resp: Response = serde_json::from_str(&line)?;
        if let Some(err) = resp.error {
            anyhow::bail!("rpc error {}: {}", err.code, err.message);
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    pub async fn status(&mut self) -> anyhow::Result<StatusResult> {
        Ok(serde_json::from_value(self.call(methods::STATUS, json!({})).await?)?)
    }
}
```

`crates/rat-cli/src/main.rs`:

```rust
mod client;
mod doctor;
mod install;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use serde_json::Value;

use rat_proto::{methods, Event, NewEvent};

/// RATO control CLI.
#[derive(Parser)]
#[command(name = "rat", version, about = "RATO control CLI")]
struct Cli {
    /// Daemon socket (default: $XDG_RUNTIME_DIR/rato/ratd.sock)
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show daemon status
    Status,
    /// Append an event (used by shell hooks and for testing)
    Emit {
        kind: String,
        #[arg(long, default_value = "cli")]
        source: String,
        /// JSON payload
        #[arg(long)]
        payload: Option<String>,
    },
    /// Inspect events
    Events {
        #[command(subcommand)]
        cmd: EventsCmd,
    },
    /// Install the user-level systemd service
    Install {
        /// Write the unit but do not run systemctl (for tests/CI)
        #[arg(long)]
        no_systemctl: bool,
        /// Explicit path to the ratd binary (default: sibling of this binary)
        #[arg(long)]
        ratd_path: Option<PathBuf>,
    },
    /// Check the local installation
    Doctor,
}

#[derive(Subcommand)]
enum EventsCmd {
    /// Show the most recent events
    Recent {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(rat_core::paths::socket_path);

    match cli.cmd {
        Cmd::Status => {
            let mut c = client::Client::connect(&socket).await?;
            let s = c.status().await?;
            println!("ratd {} (proto {})", s.version, s.proto_version);
            println!("uptime: {}s", s.uptime_ms / 1000);
            println!("events: {}", s.event_count);
            println!("db: {}", s.db_path);
        }
        Cmd::Emit { kind, source, payload } => {
            let payload: Value = match payload {
                Some(s) => serde_json::from_str(&s).context("--payload must be valid JSON")?,
                None => Value::Null,
            };
            let mut c = client::Client::connect(&socket).await?;
            let ev = NewEvent { kind, source, payload, ..Default::default() };
            let appended: Event =
                serde_json::from_value(c.call(methods::EVENTS_APPEND, serde_json::to_value(ev)?).await?)?;
            println!("{} {} {}", appended.id, appended.ts, appended.kind);
        }
        Cmd::Events { cmd: EventsCmd::Recent { limit } } => {
            let mut c = client::Client::connect(&socket).await?;
            let events: Vec<Event> = serde_json::from_value(
                c.call(methods::EVENTS_RECENT, serde_json::json!({ "limit": limit })).await?,
            )?;
            for e in events {
                let payload = if e.payload.is_null() { String::new() } else { e.payload.to_string() };
                println!("{}  {:<20} {:<10} {}", e.ts, e.kind, e.source, payload);
            }
        }
        Cmd::Install { no_systemctl, ratd_path } => install::install(no_systemctl, ratd_path)?,
        Cmd::Doctor => doctor::doctor(&socket).await?,
    }
    Ok(())
}
```

Create placeholder modules so this compiles before Task 9 (`install`/`doctor` are referenced):

`crates/rat-cli/src/install.rs`:

```rust
use std::path::PathBuf;

pub fn install(_no_systemctl: bool, _ratd_path: Option<PathBuf>) -> anyhow::Result<()> {
    anyhow::bail!("implemented in Task 9")
}
```

`crates/rat-cli/src/doctor.rs`:

```rust
use std::path::Path;

pub async fn doctor(_socket: &Path) -> anyhow::Result<()> {
    anyhow::bail!("implemented in Task 9")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p rat-cli`
Expected: the two tests in `cli.rs` PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/rat-cli && git commit -m "feat(cli): rat status/emit/events over NDJSON-RPC with hello handshake"
```

---

### Task 9: rat install + doctor + systemd unit

**Files:**
- Modify: `crates/rat-cli/src/install.rs`, `crates/rat-cli/src/doctor.rs`
- Modify: `crates/rat-cli/tests/cli.rs` (add tests)
- Create: `packaging/shell/rat-init.sh`

- [ ] **Step 1: Write the failing tests (append to `crates/rat-cli/tests/cli.rs`)**

```rust
#[test]
fn install_writes_unit_file_pointing_at_ratd() {
    let tmp = tempfile::tempdir().unwrap();
    let fake_ratd = tmp.path().join("ratd");
    std::fs::write(&fake_ratd, "#!/bin/sh\n").unwrap();
    let config = tmp.path().join("config");

    Command::cargo_bin("rat").unwrap()
        .env("XDG_CONFIG_HOME", &config)
        .args(["install", "--no-systemctl", "--ratd-path", fake_ratd.to_str().unwrap()])
        .assert()
        .success();

    let unit = config.join("systemd/user/ratd.service");
    let contents = std::fs::read_to_string(&unit).unwrap();
    assert!(contents.contains(&format!("ExecStart={}", fake_ratd.display())));
    assert!(contents.contains("Restart=on-failure"));
    assert!(contents.contains("WantedBy=default.target"));
}

#[test]
fn install_refuses_when_ratd_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");

    Command::cargo_bin("rat").unwrap()
        .env("XDG_CONFIG_HOME", &config)
        .args(["install", "--no-systemctl", "--ratd-path", "/nonexistent/ratd"])
        .assert()
        .failure()
        .stderr(contains("ratd not found"));
}

#[test]
fn doctor_reports_daemon_state_without_failing() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = start_daemon(tmp.path());

    Command::cargo_bin("rat").unwrap()
        .args(["--socket", socket.to_str().unwrap(), "doctor"])
        .assert()
        .success()
        .stdout(contains("daemon").and(contains("[ok]")));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rat-cli`
Expected: the three new tests FAIL (`bail!("implemented in Task 9")`).

- [ ] **Step 3: Implement install**

Replace `crates/rat-cli/src/install.rs`:

```rust
use std::path::{Path, PathBuf};

use anyhow::Context;

pub fn config_home() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME").expect("HOME not set")).join(".config"),
    }
}

fn unit_contents(ratd: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=RATO daemon (ratd)\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={}\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         Environment=RAT_LOG=info\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        ratd.display()
    )
}

pub fn install(no_systemctl: bool, ratd_path: Option<PathBuf>) -> anyhow::Result<()> {
    let ratd = match ratd_path {
        Some(p) => p,
        None => std::env::current_exe()?
            .parent()
            .context("rat binary has no parent directory")?
            .join("ratd"),
    };
    anyhow::ensure!(
        ratd.exists(),
        "ratd not found at {} — build it first (cargo build --release) or pass --ratd-path",
        ratd.display()
    );

    let unit_dir = config_home().join("systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    let unit_path = unit_dir.join("ratd.service");
    std::fs::write(&unit_path, unit_contents(&ratd))?;
    println!("wrote {}", unit_path.display());

    if no_systemctl {
        println!("skipped systemctl; run: systemctl --user daemon-reload && systemctl --user enable --now ratd.service");
        return Ok(());
    }
    run_systemctl(&["--user", "daemon-reload"])?;
    run_systemctl(&["--user", "enable", "--now", "ratd.service"])?;
    println!("ratd enabled and started — check: systemctl --user status ratd");
    Ok(())
}

fn run_systemctl(args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(args)
        .status()
        .with_context(|| format!("running systemctl {args:?}"))?;
    anyhow::ensure!(status.success(), "systemctl {args:?} failed");
    Ok(())
}
```

- [ ] **Step 4: Implement doctor**

Replace `crates/rat-cli/src/doctor.rs`:

```rust
use std::path::Path;

/// Prints [ok]/[warn]/[fail] lines. Always exits 0 — doctor reports, it does not gate.
pub async fn doctor(socket: &Path) -> anyhow::Result<()> {
    match crate::client::Client::connect(socket).await {
        Ok(mut c) => match c.status().await {
            Ok(s) => println!(
                "[ok]   daemon: ratd {} at {} ({} events)",
                s.version,
                socket.display(),
                s.event_count
            ),
            Err(e) => println!("[fail] daemon: connected but status failed: {e}"),
        },
        Err(_) => println!("[warn] daemon: not reachable at {} (is ratd running?)", socket.display()),
    }

    let db = rat_core::paths::db_path();
    if db.exists() {
        println!("[ok]   db: {}", db.display());
    } else {
        println!("[warn] db: {} missing (created on first daemon start)", db.display());
    }

    let unit = crate::install::config_home().join("systemd/user/ratd.service");
    if unit.exists() {
        println!("[ok]   systemd: {}", unit.display());
    } else {
        println!("[warn] systemd: unit not installed (run `rat install`)");
    }

    for (bin, arg) in [("git", "--version"), ("tmux", "-V")] {
        match std::process::Command::new(bin).arg(arg).output() {
            Ok(o) if o.status.success() => {
                let first = String::from_utf8_lossy(&o.stdout);
                println!("[ok]   {}: {}", bin, first.lines().next().unwrap_or("").trim());
            }
            _ => println!("[warn] {bin}: not found (needed from M1/M4 onward)"),
        }
    }
    Ok(())
}
```

`packaging/shell/rat-init.sh`:

```sh
# RATO shell integration (M0): a plain alias. Source from ~/.bashrc / ~/.zshrc:
#   source ~/rato/packaging/shell/rat-init.sh
alias rat="$HOME/rato/target/release/rat"
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p rat-cli`
Expected: all 5 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/rat-cli packaging && git commit -m "feat(cli): rat install (systemd --user unit) and rat doctor"
```

---

### Task 10: Acceptance pass + real install

- [ ] **Step 1: Full workspace test + lint**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all tests pass; fix any clippy findings (mechanical only — no behavior changes).

- [ ] **Step 2: Release build + live install (real machine)**

```bash
cargo build --release --workspace
./target/release/rat install
systemctl --user status ratd --no-pager
```

Expected: unit active (running). If this environment has no systemd user session, run `./target/release/rat install --no-systemctl` and start `ratd` manually in the background instead — note which path was taken in the commit message.

- [ ] **Step 3: Acceptance checks (M0 criteria from ARCHITECTURE.md §18)**

```bash
./target/release/rat status                 # round-trips: version, uptime, event count
./target/release/rat emit m0_acceptance --payload '{"ok":true}'
./target/release/rat events recent          # shows daemon_started + m0_acceptance
systemctl --user restart ratd 2>/dev/null || true
./target/release/rat events recent          # events persisted across restart
./target/release/rat doctor
```

Expected: status prints; both events listed; events survive restart (new `daemon_started` appended); doctor shows `[ok]` daemon/db/systemd lines.

- [ ] **Step 4: Commit + tag**

```bash
git add -A && git commit -m "chore: M0 acceptance pass" --allow-empty
git tag m0-spine
```

---

## Self-review notes

- **Spec coverage (M0 row of §18):** workspace ✔ (T1), `ratd`+`rat` ✔ (T7/T8), UDS RPC + hello ✔ (T6), SQLite + migrations + events ✔ (T4/T5), systemd units + install/doctor ✔ (T9), shell alias ✔ (T9 `rat-init.sh`), acceptance ✔ (T10).
- **Type consistency:** `Store::open(&Path, Arc<dyn Clock>)` used identically in T5 tests, T6 test harness, T7 main, T8 test harness. `ServerCtx { store, started, db_path }` identical in T6/T7/T8. `NewEvent`/`Event`/`StatusResult` field names match between proto (T3), store SQL (T5), dispatch (T6), and CLI prints (T8).
- **Known deliberate scope cuts (later milestones):** no event-bus pub/sub on the socket yet (M1), no sd_notify watchdog (M8), no config.toml (M1+), `rat emit` doubles as the shell-hook transport until M1's `rat shell-init`.
