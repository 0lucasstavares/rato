//! Feature-gated ScreenCast portal source placeholder.
//!
//! TODO(operator): wire ashpd + PipeWire frame capture here once the live
//! desktop/system libraries are available. Until then the feature reports
//! `Unavailable` instead of fabricating screen-capture capability.

use crate::screen::{ScreenResult, ScreenSource, SourceHealth};

pub struct PortalScreenSource {
    reason: String,
}

impl PortalScreenSource {
    pub fn new() -> Self {
        Self {
            reason: "screencast feature built, but portal backend is not wired".to_string(),
        }
    }
}

impl Default for PortalScreenSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenSource for PortalScreenSource {
    fn grab(&mut self) -> ScreenResult {
        ScreenResult::Unavailable
    }

    fn health(&self) -> SourceHealth {
        SourceHealth::Unavailable(self.reason.clone())
    }
}
