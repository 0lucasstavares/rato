use std::collections::HashMap;

use rat_core::id::new_id;
use rat_proto::WorkSession;

/// 25 minutes, per ARCHITECTURE.md §9.
pub const DEFAULT_GAP_MS: i64 = 25 * 60 * 1000;

#[derive(Debug, Clone, PartialEq)]
pub enum SessionUpdate {
    Open(WorkSession),
    Touch { id: String, last_activity: i64, commands: u32 },
    Close { id: String, ended: i64 },
}

#[derive(Debug, Clone)]
struct OpenSession {
    id: String,
    last_activity: i64,
    commands: u32,
}

/// Pure work-session grouping: activity events per project, closed after a
/// `gap_ms` silence. Sessions end at their last activity, not at detection time.
pub struct Sessionizer {
    gap_ms: i64,
    open: HashMap<String, OpenSession>,
}

impl Sessionizer {
    pub fn new(gap_ms: i64) -> Self {
        Self { gap_ms, open: HashMap::new() }
    }

    /// Re-adopt sessions left open by a previous daemon run.
    pub fn preload(&mut self, sessions: &[WorkSession]) {
        for ws in sessions {
            self.open.insert(
                ws.project_id.clone(),
                OpenSession { id: ws.id.clone(), last_activity: ws.last_activity, commands: ws.commands },
            );
        }
    }

    pub fn on_activity(&mut self, project_id: &str, ts: i64, is_command: bool) -> Vec<SessionUpdate> {
        let mut updates = Vec::new();
        match self.open.get_mut(project_id) {
            Some(open) if ts - open.last_activity <= self.gap_ms => {
                open.last_activity = ts;
                if is_command {
                    open.commands += 1;
                }
                updates.push(SessionUpdate::Touch {
                    id: open.id.clone(),
                    last_activity: open.last_activity,
                    commands: open.commands,
                });
            }
            stale => {
                if let Some(old) = stale {
                    updates.push(SessionUpdate::Close { id: old.id.clone(), ended: old.last_activity });
                }
                let ws = WorkSession {
                    id: new_id(),
                    project_id: project_id.to_string(),
                    started: ts,
                    last_activity: ts,
                    ended: None,
                    commands: u32::from(is_command),
                };
                self.open.insert(
                    project_id.to_string(),
                    OpenSession { id: ws.id.clone(), last_activity: ts, commands: ws.commands },
                );
                updates.push(SessionUpdate::Open(ws));
            }
        }
        updates
    }

    /// Close every session whose silence exceeds the gap.
    pub fn tick(&mut self, now: i64) -> Vec<SessionUpdate> {
        let gap = self.gap_ms;
        let stale: Vec<String> = self
            .open
            .iter()
            .filter(|(_, s)| now - s.last_activity > gap)
            .map(|(p, _)| p.clone())
            .collect();
        let mut updates = Vec::new();
        for project in stale {
            if let Some(s) = self.open.remove(&project) {
                updates.push(SessionUpdate::Close { id: s.id, ended: s.last_activity });
            }
        }
        updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAP: i64 = 25 * 60 * 1000;

    fn opens(updates: &[SessionUpdate]) -> Vec<&WorkSession> {
        updates
            .iter()
            .filter_map(|u| match u {
                SessionUpdate::Open(ws) => Some(ws),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn single_burst_is_one_session() {
        let mut s = Sessionizer::new(GAP);
        let first = s.on_activity("p1", 1_000, true);
        assert_eq!(opens(&first).len(), 1);
        let second = s.on_activity("p1", 60_000, true);
        assert!(matches!(second[0], SessionUpdate::Touch { commands: 2, last_activity: 60_000, .. }));
        assert!(s.tick(60_000 + GAP).is_empty()); // exactly at gap: still open
        assert_eq!(s.tick(60_000 + GAP + 1).len(), 1); // strictly past gap: closed
    }

    #[test]
    fn gap_closes_old_and_opens_new() {
        let mut s = Sessionizer::new(GAP);
        let first_id = match &s.on_activity("p1", 1_000, true)[0] {
            SessionUpdate::Open(ws) => ws.id.clone(),
            _ => panic!(),
        };
        let updates = s.on_activity("p1", 1_000 + GAP + 1, true);
        assert_eq!(updates.len(), 2);
        assert!(matches!(&updates[0], SessionUpdate::Close { id, ended: 1_000 } if *id == first_id));
        assert_eq!(opens(&updates).len(), 1);
        assert_ne!(opens(&updates)[0].id, first_id);
    }

    #[test]
    fn two_projects_run_parallel_sessions() {
        let mut s = Sessionizer::new(GAP);
        s.on_activity("p1", 1_000, true);
        s.on_activity("p2", 2_000, true);
        assert!(matches!(s.on_activity("p1", 3_000, true)[0], SessionUpdate::Touch { .. }));
        assert!(matches!(s.on_activity("p2", 4_000, false)[0], SessionUpdate::Touch { commands: 1, .. }));
    }

    #[test]
    fn tick_closes_only_stale_sessions() {
        let mut s = Sessionizer::new(GAP);
        s.on_activity("old", 1_000, true);
        s.on_activity("fresh", 1_000 + GAP, true);
        let updates = s.tick(1_000 + GAP + 1);
        assert_eq!(updates.len(), 1);
        assert!(matches!(&updates[0], SessionUpdate::Close { ended: 1_000, .. }));
        // fresh still open: next activity touches it
        assert!(matches!(s.on_activity("fresh", 1_000 + GAP + 2, false)[0], SessionUpdate::Touch { .. }));
    }

    #[test]
    fn non_command_activity_does_not_increment_commands() {
        let mut s = Sessionizer::new(GAP);
        s.on_activity("p1", 1_000, false);
        let u = s.on_activity("p1", 2_000, false);
        assert!(matches!(u[0], SessionUpdate::Touch { commands: 0, .. }));
    }

    #[test]
    fn preload_adopts_open_sessions_across_restart() {
        let mut s = Sessionizer::new(GAP);
        s.preload(&[WorkSession {
            id: "s-old".into(),
            project_id: "p1".into(),
            started: 500,
            last_activity: 1_000,
            ended: None,
            commands: 4,
        }]);
        let u = s.on_activity("p1", 2_000, true);
        assert!(matches!(&u[0], SessionUpdate::Touch { id, commands: 5, .. } if id == "s-old"));
    }
}
