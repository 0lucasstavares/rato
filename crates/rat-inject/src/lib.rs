//! Pure injection ceremony core for M7.
//!
//! Real injectors (tmux, portal/libei, ydotool, xdotool) plug into this later.
//! This crate models the safety gate: exact bytes, target recheck, expiry, away
//! mode, bracketed paste, and Enter as a separate approved action.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectionRequest {
    pub approval_id: String,
    pub target: InjectionTarget,
    pub exact_bytes: String,
    pub include_enter: bool,
    pub created_ms: i64,
    pub expires_at_ms: i64,
    pub expected_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InjectionTarget {
    TmuxPane { target: String },
    X11Window { window_id: String },
    WaylandRemote { session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSnapshot {
    pub exists: bool,
    pub current_command: Option<String>,
    pub focused: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModeState {
    Active,
    Away,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlannedAction {
    PasteBracketed { bytes: String },
    PressEnter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedInjection {
    request: InjectionRequest,
    approved_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedInjection {
    request: InjectionRequest,
    actions: Vec<PlannedAction>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InjectionError {
    #[error("approval expired")]
    Expired,
    #[error("away mode blocks injection")]
    Away,
    #[error("target no longer exists")]
    MissingTarget,
    #[error("target command changed")]
    CommandChanged,
    #[error("focused window does not match target")]
    FocusMismatch,
}

impl InjectionRequest {
    pub fn approve(self, now_ms: i64) -> Result<ApprovedInjection, InjectionError> {
        if now_ms > self.expires_at_ms {
            return Err(InjectionError::Expired);
        }
        Ok(ApprovedInjection {
            request: self,
            approved_at_ms: now_ms,
        })
    }
}

impl ApprovedInjection {
    pub fn verify(
        self,
        now_ms: i64,
        mode: ModeState,
        snapshot: TargetSnapshot,
    ) -> Result<VerifiedInjection, InjectionError> {
        if now_ms > self.request.expires_at_ms {
            return Err(InjectionError::Expired);
        }
        if mode == ModeState::Away {
            return Err(InjectionError::Away);
        }
        if !snapshot.exists {
            return Err(InjectionError::MissingTarget);
        }
        if snapshot.current_command.as_deref() != Some(self.request.expected_command.as_str()) {
            return Err(InjectionError::CommandChanged);
        }
        if matches!(self.request.target, InjectionTarget::X11Window { .. })
            && snapshot.focused != Some(true)
        {
            return Err(InjectionError::FocusMismatch);
        }

        let mut actions = vec![PlannedAction::PasteBracketed {
            bytes: bracketed_paste(&self.request.exact_bytes),
        }];
        if self.request.include_enter {
            actions.push(PlannedAction::PressEnter);
        }
        Ok(VerifiedInjection {
            request: self.request,
            actions,
        })
    }
}

impl VerifiedInjection {
    pub fn actions(&self) -> &[PlannedAction] {
        &self.actions
    }

    pub fn request(&self) -> &InjectionRequest {
        &self.request
    }
}

pub fn bracketed_paste(bytes: &str) -> String {
    format!("\x1b[200~{bytes}\x1b[201~")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(include_enter: bool) -> InjectionRequest {
        InjectionRequest {
            approval_id: "ap1".into(),
            target: InjectionTarget::TmuxPane {
                target: "sess:0.0".into(),
            },
            exact_bytes: "cargo test".into(),
            include_enter,
            created_ms: 1_000,
            expires_at_ms: 11_000,
            expected_command: "bash".into(),
        }
    }

    fn snapshot() -> TargetSnapshot {
        TargetSnapshot {
            exists: true,
            current_command: Some("bash".into()),
            focused: None,
        }
    }

    #[test]
    fn approved_unexpired_request_verifies_to_bracketed_paste_without_enter() {
        let verified = req(false)
            .approve(2_000)
            .unwrap()
            .verify(3_000, ModeState::Active, snapshot())
            .unwrap();

        assert_eq!(
            verified.actions(),
            &[PlannedAction::PasteBracketed {
                bytes: "\x1b[200~cargo test\x1b[201~".into()
            }]
        );
    }

    #[test]
    fn enter_is_a_separate_planned_action() {
        let verified = req(true)
            .approve(2_000)
            .unwrap()
            .verify(3_000, ModeState::Active, snapshot())
            .unwrap();

        assert_eq!(verified.actions().len(), 2);
        assert!(matches!(verified.actions()[1], PlannedAction::PressEnter));
    }

    #[test]
    fn expired_request_cannot_be_approved_or_verified() {
        assert_eq!(req(false).approve(12_000), Err(InjectionError::Expired));
        let approved = req(false).approve(2_000).unwrap();
        assert_eq!(
            approved.verify(12_000, ModeState::Active, snapshot()),
            Err(InjectionError::Expired)
        );
    }

    #[test]
    fn away_mode_blocks_even_after_approval() {
        let approved = req(false).approve(2_000).unwrap();
        assert_eq!(
            approved.verify(3_000, ModeState::Away, snapshot()),
            Err(InjectionError::Away)
        );
    }

    #[test]
    fn jit_recheck_aborts_on_missing_target_or_command_change() {
        let approved = req(false).approve(2_000).unwrap();
        assert_eq!(
            approved.clone().verify(
                3_000,
                ModeState::Active,
                TargetSnapshot {
                    exists: false,
                    current_command: Some("bash".into()),
                    focused: None,
                },
            ),
            Err(InjectionError::MissingTarget)
        );
        assert_eq!(
            approved.verify(
                3_000,
                ModeState::Active,
                TargetSnapshot {
                    exists: true,
                    current_command: Some("vim".into()),
                    focused: None,
                },
            ),
            Err(InjectionError::CommandChanged)
        );
    }

    #[test]
    fn x11_requires_focus_match() {
        let mut request = req(false);
        request.target = InjectionTarget::X11Window {
            window_id: "0xabc".into(),
        };
        let approved = request.approve(2_000).unwrap();
        assert_eq!(
            approved.verify(
                3_000,
                ModeState::Active,
                TargetSnapshot {
                    exists: true,
                    current_command: Some("bash".into()),
                    focused: Some(false),
                },
            ),
            Err(InjectionError::FocusMismatch)
        );
    }
}
