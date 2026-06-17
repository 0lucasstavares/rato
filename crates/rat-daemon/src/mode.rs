use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use rat_proto::{ModeState, NewEvent};
use serde_json::json;

use crate::ingest::Ingest;

/// 15 minutes per ARCHITECTURE.md — Away starts after this much idle.
pub const AWAY_THRESHOLD_MS: i64 = 15 * 60 * 1000;

/// Active/Away state. M1 tracks and exposes it; enforcement (blocking
/// injection/installs) arrives with the approval engine in M4.
pub struct ModeManager {
    away: AtomicBool,
    since_ms: AtomicI64,
    /// -1 = unknown (no probe data yet)
    idle_ms: AtomicI64,
    /// fallback signal when no D-Bus idle interface exists
    last_event_ms: AtomicI64,
}

impl ModeManager {
    pub fn new(now: i64) -> Self {
        Self {
            away: AtomicBool::new(false),
            since_ms: AtomicI64::new(now),
            idle_ms: AtomicI64::new(-1),
            last_event_ms: AtomicI64::new(now),
        }
    }

    pub fn note_activity(&self, ts: i64) {
        self.last_event_ms.store(ts, Ordering::Relaxed);
    }

    pub fn state(&self) -> ModeState {
        let idle = self.idle_ms.load(Ordering::Relaxed);
        ModeState {
            mode: if self.away.load(Ordering::Relaxed) {
                "away"
            } else {
                "active"
            }
            .to_string(),
            since_ms: self.since_ms.load(Ordering::Relaxed),
            idle_ms: (idle >= 0).then_some(idle),
        }
    }

    /// Feed one idle measurement; returns a mode_changed event on transition.
    pub fn update(&self, probe_idle_ms: Option<i64>, now: i64) -> Option<NewEvent> {
        let idle = probe_idle_ms.unwrap_or(now - self.last_event_ms.load(Ordering::Relaxed));
        self.idle_ms.store(idle.max(0), Ordering::Relaxed);
        let should_be_away = idle >= AWAY_THRESHOLD_MS;
        let was_away = self.away.swap(should_be_away, Ordering::Relaxed);
        if was_away == should_be_away {
            return None;
        }
        self.since_ms.store(now, Ordering::Relaxed);
        let mode = if should_be_away { "away" } else { "active" };
        tracing::info!("mode changed: {mode} (idle {idle} ms)");
        Some(NewEvent {
            kind: "mode_changed".into(),
            source: "idle".into(),
            payload: json!({"mode": mode, "idle_ms": idle}),
            ..Default::default()
        })
    }
}

/// 30 s loop: probe idle, update mode, persist transitions.
pub async fn run(
    mode: Arc<ModeManager>,
    ingest: Arc<Ingest>,
    clock: Arc<dyn rat_core::clock::Clock>,
) {
    let probe = rat_sensors::idle::IdleProbe::connect().await;
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let idle = probe.idle_ms().await;
        if let Some(ev) = mode.update(idle, clock.now_ms()) {
            if let Err(e) = ingest.ingest(ev).await {
                tracing::warn!("mode event ingest failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_to_away_after_threshold_and_back() {
        let m = ModeManager::new(0);
        assert!(m.update(Some(1_000), 1_000).is_none());
        assert_eq!(m.state().mode, "active");

        let ev = m
            .update(Some(AWAY_THRESHOLD_MS), 2_000)
            .expect("transition");
        assert_eq!(ev.payload["mode"], "away");
        assert_eq!(m.state().mode, "away");
        assert_eq!(m.state().since_ms, 2_000);

        // staying away: no repeated events
        assert!(m.update(Some(AWAY_THRESHOLD_MS + 60_000), 3_000).is_none());

        let back = m.update(Some(500), 4_000).expect("transition back");
        assert_eq!(back.payload["mode"], "active");
        assert_eq!(m.state().mode, "active");
    }

    #[test]
    fn falls_back_to_last_event_when_no_probe() {
        let m = ModeManager::new(0);
        m.note_activity(10_000);
        assert!(m.update(None, 20_000).is_none()); // idle 10s
        assert_eq!(m.state().idle_ms, Some(10_000));
        let ev = m
            .update(None, 10_000 + AWAY_THRESHOLD_MS)
            .expect("away via fallback");
        assert_eq!(ev.payload["mode"], "away");
    }
}
