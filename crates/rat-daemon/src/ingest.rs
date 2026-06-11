use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use rat_core::clock::Clock;
use rat_proto::{Event, NewEvent, NewObservation, Project};
use rat_store::error::StoreError;
use rat_store::store::Store;

use crate::sessionizer::{SessionUpdate, Sessionizer};

/// Event kinds that count as project activity for the sessionizer.
const ACTIVITY_KINDS: &[&str] = &["shell_cmd", "git_head", "proc_started"];

/// Central ingestion pipeline: every event (RPC or internal sensor) flows
/// through here — project attribution, persistence, observation derivation,
/// session grouping.
pub struct Ingest {
    store: Store,
    clock: Arc<dyn Clock>,
    sessionizer: Mutex<Sessionizer>,
    /// cwd → resolved project root (None = not inside any repo).
    project_cache: Mutex<HashMap<PathBuf, Option<PathBuf>>>,
}

impl Ingest {
    pub fn new(store: Store, clock: Arc<dyn Clock>, sessionizer: Sessionizer) -> Self {
        Self { store, clock, sessionizer: Mutex::new(sessionizer), project_cache: Mutex::new(HashMap::new()) }
    }

    /// Returns Ok(None) when the event is deliberately dropped (loop guard).
    pub async fn ingest(&self, mut ev: NewEvent) -> Result<Option<Event>, StoreError> {
        // Loop guard: our own shell hooks calling `rat emit*` must not echo forever.
        if ev.kind == "shell_cmd" {
            if let Some(cmd) = ev.payload.get("cmd").and_then(|v| v.as_str()) {
                let trimmed = cmd.trim_start();
                if trimmed.starts_with("rat emit") || trimmed.contains("/rat emit") {
                    return Ok(None);
                }
            }
        }

        let project = match ev.payload.get("cwd").and_then(|v| v.as_str()) {
            Some(cwd) if ev.project_id.is_none() => self.resolve_project(Path::new(cwd)).await?,
            _ => None,
        };
        if let Some(p) = &project {
            ev.project_id = Some(p.id.clone());
        }

        let event = self.store.append(ev).await?;

        if let Some(obs) = derive_observation(&event) {
            self.store.add_observation(obs).await?;
        }

        if let (Some(project_id), true) =
            (event.project_id.clone(), ACTIVITY_KINDS.contains(&event.kind.as_str()))
        {
            let updates = {
                let mut sz = self.sessionizer.lock().await;
                sz.on_activity(&project_id, event.ts, event.kind == "shell_cmd")
            };
            self.apply(updates).await?;
        }

        Ok(Some(event))
    }

    /// Close sessions that went silent past the gap. Called on a timer.
    pub async fn tick(&self) -> Result<(), StoreError> {
        let now = self.clock.now_ms();
        let updates = {
            let mut sz = self.sessionizer.lock().await;
            sz.tick(now)
        };
        self.apply(updates).await
    }

    async fn apply(&self, updates: Vec<SessionUpdate>) -> Result<(), StoreError> {
        for u in updates {
            match u {
                SessionUpdate::Open(ws) => self.store.session_open(ws).await?,
                SessionUpdate::Touch { id, last_activity, commands } => {
                    self.store.session_touch(id, last_activity, commands).await?
                }
                SessionUpdate::Close { id, ended } => self.store.session_close(id, ended).await?,
            }
        }
        Ok(())
    }

    async fn resolve_project(&self, cwd: &Path) -> Result<Option<Project>, StoreError> {
        let root = {
            let mut cache = self.project_cache.lock().await;
            match cache.get(cwd) {
                Some(cached) => cached.clone(),
                None => {
                    let found = find_git_root(cwd);
                    cache.insert(cwd.to_path_buf(), found.clone());
                    found
                }
            }
        };
        match root {
            Some(root) => {
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unnamed".to_string());
                Ok(Some(
                    self.store.upsert_project(root.to_string_lossy().into_owned(), name).await?,
                ))
            }
            None => Ok(None),
        }
    }
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start;
    for _ in 0..20 {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
    None
}

fn derive_observation(event: &Event) -> Option<NewObservation> {
    let content = match event.kind.as_str() {
        "shell_cmd" => event.payload.get("cmd")?.as_str()?.to_string(),
        "clipboard_text" | "clipboard_redacted" => event.payload.get("text")?.as_str()?.to_string(),
        "git_head" => {
            let branch = event.payload.get("branch").and_then(|v| v.as_str()).unwrap_or("detached");
            let commit = event.payload.get("commit").and_then(|v| v.as_str()).unwrap_or("?");
            format!("checkout {branch}@{}", &commit[..commit.len().min(12)])
        }
        _ => return None,
    };
    let mut meta = event.payload.clone();
    if let Some(obj) = meta.as_object_mut() {
        obj.remove("cmd");
        obj.remove("text");
    }
    Some(NewObservation {
        event_id: Some(event.id.clone()),
        kind: event.kind.clone(),
        project_id: event.project_id.clone(),
        content,
        meta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionizer::DEFAULT_GAP_MS;
    use rat_core::clock::FakeClock;
    use serde_json::json;

    fn shell_cmd(cmd: &str, cwd: &Path) -> NewEvent {
        NewEvent {
            kind: "shell_cmd".into(),
            source: "shell".into(),
            payload: json!({"cmd": cmd, "cwd": cwd.to_string_lossy(), "exit": 0, "duration_ms": 12}),
            ..Default::default()
        }
    }

    async fn setup(tmp: &Path) -> (Ingest, std::sync::Arc<FakeClock>, PathBuf) {
        let clock = FakeClock::at(1_000);
        let store = Store::open(&tmp.join("t.db"), clock.clone()).unwrap();
        let ingest = Ingest::new(store, clock.clone(), Sessionizer::new(DEFAULT_GAP_MS));
        let repo = tmp.join("myproj");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let sub = repo.join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();
        (ingest, clock, repo)
    }

    #[tokio::test]
    async fn shell_cmd_creates_project_observation_and_session() {
        let tmp = tempfile::tempdir().unwrap();
        let (ingest, _clock, repo) = setup(tmp.path()).await;

        let ev = ingest.ingest(shell_cmd("cargo test", &repo.join("src/deep"))).await.unwrap().unwrap();
        assert!(ev.project_id.is_some());

        let store_view = ingest.store.clone();
        let projects = store_view.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "myproj");

        let obs = store_view.recent_observations(10, Some("shell_cmd".into())).await.unwrap();
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].content, "cargo test");
        assert_eq!(obs[0].meta["exit"], 0);
        assert!(obs[0].meta.get("cmd").is_none());

        let sessions = store_view.open_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].commands, 1);
    }

    #[tokio::test]
    async fn burst_shares_session_and_gap_opens_new_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (ingest, clock, repo) = setup(tmp.path()).await;

        ingest.ingest(shell_cmd("ls", &repo)).await.unwrap();
        clock.advance(60_000);
        ingest.ingest(shell_cmd("cargo build", &repo)).await.unwrap();
        let store = ingest.store.clone();
        assert_eq!(store.open_sessions().await.unwrap().len(), 1);
        assert_eq!(store.open_sessions().await.unwrap()[0].commands, 2);

        clock.advance(DEFAULT_GAP_MS + 1);
        ingest.ingest(shell_cmd("git status", &repo)).await.unwrap();
        let all = store.recent_sessions(10).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(store.open_sessions().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cwd_outside_any_repo_stores_event_without_project() {
        let tmp = tempfile::tempdir().unwrap();
        let (ingest, _clock, _repo) = setup(tmp.path()).await;
        let outside = tmp.path().join("nowhere");
        std::fs::create_dir_all(&outside).unwrap();

        let ev = ingest.ingest(shell_cmd("echo hi", &outside)).await.unwrap().unwrap();
        assert!(ev.project_id.is_none());
        assert_eq!(ingest.store.open_sessions().await.unwrap().len(), 0);
        // observation still derived
        assert_eq!(ingest.store.recent_observations(10, None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rat_emit_commands_are_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let (ingest, _clock, repo) = setup(tmp.path()).await;
        let dropped = ingest.ingest(shell_cmd("rat emit foo --payload '{}'", &repo)).await.unwrap();
        assert!(dropped.is_none());
        assert_eq!(ingest.store.count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn tick_closes_stale_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let (ingest, clock, repo) = setup(tmp.path()).await;
        ingest.ingest(shell_cmd("ls", &repo)).await.unwrap();
        clock.advance(DEFAULT_GAP_MS + 1);
        ingest.tick().await.unwrap();
        assert_eq!(ingest.store.open_sessions().await.unwrap().len(), 0);
    }
}
