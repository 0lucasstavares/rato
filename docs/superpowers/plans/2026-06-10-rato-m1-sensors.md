# RATO M1 — Cheap Sensors Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans. Executed inline by the plan author in the same session — interface contracts and test specs below are binding; full code lives in the commits.

**Goal:** M1 per `docs/ARCHITECTURE.md` §18 — shell hooks, procfs watcher, git watcher, clipboard sensor, idle/Away mode, project registry, sessionizer v1.

**Acceptance:** commands/git/clipboard land as observations; Away triggers at 15 min idle; sessions form correctly on synthetic timelines (FakeClock tests).

**Environment facts (probed):** GNOME 4x on Wayland (`ubuntu:GNOME`, `wayland-0`, XWayland `:0`). Mutter IdleMonitor D-Bus works. No wl-paste/xclip. → clipboard = `arboard` (default-features off + `wayland-data-control`), idle = `zbus` → `org.gnome.Mutter.IdleMonitor.GetIdletime`, fallback `org.freedesktop.ScreenSaver.GetSessionIdleTime`, fallback last-sensor-event.

---

## Task 1: Schema v2 + store repos

- Migration v2 in `rat-store/src/db.rs` (append to `MIGRATIONS`):

```sql
CREATE TABLE projects (
    id TEXT PRIMARY KEY, root_path TEXT UNIQUE NOT NULL, name TEXT NOT NULL,
    first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL);
CREATE TABLE work_sessions (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
    started INTEGER NOT NULL, last_activity INTEGER NOT NULL,
    ended INTEGER, commands INTEGER NOT NULL DEFAULT 0);
CREATE INDEX idx_sessions_proj ON work_sessions(project_id, started);
CREATE TABLE observations (
    id TEXT PRIMARY KEY, event_id TEXT, ts INTEGER NOT NULL, kind TEXT NOT NULL,
    project_id TEXT, content TEXT NOT NULL, meta TEXT NOT NULL DEFAULT '{}');
CREATE INDEX idx_obs_ts ON observations(ts);
CREATE INDEX idx_obs_kind_ts ON observations(kind, ts);
```

- New `Store` methods (actor commands): `upsert_project(root_path, name) -> Project` (insert-or-touch `last_seen`), `list_projects() -> Vec<Project>`, `add_observation(NewObservation) -> Observation`, `recent_observations(limit, kind: Option<String>) -> Vec<Observation>`, `session_open(WorkSession)`, `session_touch(id, last_activity, commands)`, `session_close(id, ended)`, `recent_sessions(limit) -> Vec<WorkSession>`, `open_sessions() -> Vec<WorkSession>` (for daemon-restart recovery).
- **Tests:** migration v1→v2 upgrade (open old-schema db, reopen, user_version==2, old events intact); project upsert idempotent by root_path; session open/touch/close/recent ordering; observation round-trip incl. meta JSON.

## Task 2: Proto additions (stays PROTO_VERSION 1 — additive)

- Types: `Project`, `WorkSession`, `Observation { id, ts, kind, project_id, content, meta }`, `NewObservation`, `ModeState { mode: String /*active|away*/, since_ms: i64, idle_ms: Option<i64> }`, `ObsRecentParams { limit (default 50), kind: Option<String> }`.
- Methods: `observations.recent`, `projects.list`, `sessions.recent`, `mode.get`.
- **Tests:** serde defaults round-trip.

## Task 3: Sessionizer v1 (pure, daemon module `rat-daemon/src/sessionizer.rs`)

- `Sessionizer::new(gap_ms /*25min*/)`; `on_activity(project_id, ts, is_command) -> Vec<SessionUpdate>`; `tick(now) -> Vec<SessionUpdate>`; `preload(open_sessions)` for restart recovery.
- `SessionUpdate::{Open(WorkSession), Touch{id, last_activity, commands}, Close{id, ended}}`. Session `ended = last_activity` (not close-detection time).
- **Tests (FakeClock synthetic timelines):** single burst → one session; gap > 25 min → close + new session; two projects interleaved → two parallel sessions; `tick` closes stale; commands counter increments only when `is_command`.

## Task 4: Ingest pipeline (`rat-daemon/src/ingest.rs`)

- `Ingest { store, sessionizer: Mutex<Sessionizer>, clock, project_cache: Mutex<HashMap<PathBuf, Option<Project>>> }`.
- `ingest(NewEvent) -> Event`: resolve project from `payload.cwd` (walk up ≤20 levels to dir containing `.git`; cache), set `project_id`, persist event, derive observation for kinds `shell_cmd` (content=cmd, meta {cwd,exit,duration_ms}), `clipboard_text` (content truncated 4096), `git_head` (content="checkout <branch>@<short>"), feed sessionizer (`is_command` = shell_cmd) and apply `SessionUpdate`s to store. Kinds `proc_started/proc_exited/mode_changed/daemon_started` → event only.
- Shell-hook loop guard: shell_cmd whose cmd starts with `rat emit` is dropped (event not stored).
- `events.append` RPC now routes through `ingest`.
- **Tests:** temp dir with `.git/` → shell_cmd creates project + observation + open session; same-burst second cmd touches same session; FakeClock jump past gap → new session; cwd outside any repo → event stored, no project/session.

## Task 5: rat-sensors crate

New crate `crates/rat-sensors` (deps: tokio, arboard, zbus, regex, libc, rat-proto, rat-core, tracing). Each sensor emits `NewEvent` into a `tokio::sync::mpsc::Sender<NewEvent>`.

- `proc.rs`: every 5 s scan `/proc/<pid>/comm` against dev allowlist (cargo, rustc, node, npm, pnpm, yarn, python(3), pytest, go, make, cmake, gcc, clang, docker, tsc, vitest, jest, claude, codex, aider, gemini, tmux); diff pid set → `proc_started` (payload: comm, cmdline ≤512 ch, cwd via readlink) / `proc_exited` (duration_ms). Pure diff function unit-tested with injected snapshots.
- `gitwatch.rs`: every 10 s for projects seen in last 24 h: read HEAD via files only (`.git` dir or worktree `gitdir:` file; `ref:` → branch, ref file or packed-refs → commit); on change emit `git_head` {branch, commit, cwd: root}. Unit test against a real `git init` temp repo (git is installed).
- `clipboard.rs`: std thread, arboard poll 1 s, dedupe by hash; secret filter (regex set: BEGIN…PRIVATE KEY, `AKIA[0-9A-Z]{16}`, `gh[pousr]_[A-Za-z0-9]{36,}`, `sk-[A-Za-z0-9_-]{20,}`, `(?i)(api[_-]?key|secret|token|passwd|password)["']?\s*[:=]`) → matched content becomes kind `clipboard_redacted`, content `"[redacted: secret-like]"`; clean text → `clipboard_text` truncated 4096 (meta {len, truncated}); >32 KiB skipped. Filter unit-tested with positive/negative corpus.
- `idle.rs`: `IdleProbe` — zbus Mutter `GetIdletime` (ms) → fallback ScreenSaver `GetSessionIdleTime` (s×1000) → `None`.

## Task 6: Daemon wiring + mode manager

- `mode.rs`: `ModeManager` (AtomicBool away + since); 30 s loop: idle = probe → else `now - last_event_ts`; ≥ 900 000 ms → Away, else Active; transitions emit `mode_changed` events via ingest.
- main: build Ingest (preload open sessions), spawn sensor tasks + mpsc pump → ingest, 60 s sessionizer tick, mode loop. `ServerCtx` gains `ingest`, `mode`; dispatch adds the four new methods.
- **Tests:** extend `tests/rpc.rs` — append shell_cmd with cwd of a temp git repo via RPC, then `projects.list`, `sessions.recent`, `observations.recent` reflect it; `mode.get` returns active.

## Task 7: CLI

- `rat shell-init [bash|zsh]` prints hook snippet (absolute path of current `rat` baked in; bash DEBUG-trap + PROMPT_COMMAND, zsh add-zsh-hook; both call `rat emit-shell` in a detached subshell, fail-silent).
- `rat emit-shell --cmd --cwd --exit --duration-ms` (builds JSON safely, sends `events.append`).
- `rat projects list`, `rat sessions recent [--limit]`, `rat observations recent [--limit --kind]`, `rat mode`.
- doctor: add mode/idle line and clipboard backend note.
- **Tests:** emit-shell against in-process daemon → observation + session visible via CLI; shell-init output contains `emit-shell` and the binary path.

## Task 8: Acceptance (live)

1. `cargo test --workspace` + clippy clean.
2. Release build, restart service, `rat emit-shell --cmd "cargo test" --cwd ~/rato --exit 0 --duration-ms 1200` → `rat projects list` shows `rato`; `rat sessions recent` shows open session; `rat observations recent` shows the command.
3. `rat mode` shows active + idle ms from Mutter.
4. Copy text to clipboard (via `rat`? manual) — observation `clipboard_text` appears (best-effort; GNOME data-control may require XWayland fallback — record outcome honestly).
5. Append `source ~/rato/packaging/shell/rat-init.sh` guidance to README; rat-init.sh gains `eval "$(rat shell-init bash)"`.
6. Commit + tag `m1-sensors`.
