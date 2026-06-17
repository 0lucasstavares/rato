//! SensorGate (M5 §5): tracks per-sensor health (`screen`, `ocr`) so the
//! `status` RPC / Sensors tab / `rat doctor` can report what's actually
//! available without fabricating capability. `Unavailable(reason)` is a
//! first-class, expected state in default builds.

use std::sync::Mutex;

use rat_proto::SensorHealthDto;
use rat_vision::screen::SourceHealth;

/// Shared, mutable sensor-health table. The capture loop updates `screen`
/// and `ocr` entries from `ScreenSource::health()` / `OcrEngine::health()`
/// on every tick (even when the tick early-returns due to Unavailable).
#[derive(Debug)]
pub struct SensorGate {
    screen: Mutex<SourceHealth>,
    ocr: Mutex<SourceHealth>,
}

impl Default for SensorGate {
    fn default() -> Self {
        Self {
            screen: Mutex::new(SourceHealth::Unavailable("not started".to_string())),
            ocr: Mutex::new(SourceHealth::Unavailable("not started".to_string())),
        }
    }
}

impl SensorGate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_screen(&self, health: SourceHealth) {
        *self.screen.lock().unwrap() = health;
    }

    pub fn set_ocr(&self, health: SourceHealth) {
        *self.ocr.lock().unwrap() = health;
    }

    pub fn screen(&self) -> SourceHealth {
        self.screen.lock().unwrap().clone()
    }

    pub fn ocr(&self) -> SourceHealth {
        self.ocr.lock().unwrap().clone()
    }

    /// Snapshot as wire DTOs, in `[screen, ocr]` order — for `status.sensors`
    /// and `rat doctor`.
    pub fn snapshot(&self) -> Vec<SensorHealthDto> {
        vec![
            health_to_dto("screen", self.screen()),
            health_to_dto("ocr", self.ocr()),
        ]
    }
}

fn health_to_dto(name: &str, health: SourceHealth) -> SensorHealthDto {
    match health {
        SourceHealth::Ok => SensorHealthDto {
            name: name.to_string(),
            state: "ok".to_string(),
            reason: None,
        },
        SourceHealth::Unavailable(reason) => SensorHealthDto {
            name: name.to_string(),
            state: "unavailable".to_string(),
            reason: Some(reason),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_gate_reports_unavailable_for_screen_and_ocr() {
        let gate = SensorGate::new();
        let snap = gate.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].name, "screen");
        assert_eq!(snap[0].state, "unavailable");
        assert!(snap[0].reason.is_some());
        assert_eq!(snap[1].name, "ocr");
        assert_eq!(snap[1].state, "unavailable");
    }

    #[test]
    fn set_screen_ok_reflects_in_snapshot() {
        let gate = SensorGate::new();
        gate.set_screen(SourceHealth::Ok);
        let snap = gate.snapshot();
        assert_eq!(snap[0].state, "ok");
        assert!(snap[0].reason.is_none());
    }

    #[test]
    fn set_ocr_unavailable_with_reason() {
        let gate = SensorGate::new();
        gate.set_ocr(SourceHealth::Unavailable(
            "ocr feature not built".to_string(),
        ));
        let snap = gate.snapshot();
        assert_eq!(snap[1].state, "unavailable");
        assert_eq!(snap[1].reason.as_deref(), Some("ocr feature not built"));
    }
}
