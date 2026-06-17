use crate::slug::spoken_slug;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalSnapshot {
    pub id: String,
    pub risk: i64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupState {
    pub visible_approval_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceApprovalDecision {
    Allow { approval_id: String, status: String },
    Refuse { reason: String },
}

pub fn voice_approval_decision(
    approval: &ApprovalSnapshot,
    popup: &PopupState,
    utterance: &str,
    approve: bool,
) -> VoiceApprovalDecision {
    if approval.status != "pending" {
        return refuse("approval is not pending");
    }
    if popup.visible_approval_id.as_deref() != Some(approval.id.as_str()) {
        return refuse("approval popup is not visible");
    }
    if approval.risk >= 3 {
        return refuse("R3 approvals are never voice-approvable");
    }
    let slug = spoken_slug(&approval.id);
    if !utterance.to_lowercase().contains(&slug) {
        return refuse(format!("utterance missing spoken slug {slug}"));
    }
    VoiceApprovalDecision::Allow {
        approval_id: approval.id.clone(),
        status: if approve { "approved" } else { "denied" }.to_string(),
    }
}

fn refuse(reason: impl Into<String>) -> VoiceApprovalDecision {
    VoiceApprovalDecision::Refuse {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_r2() -> ApprovalSnapshot {
        ApprovalSnapshot {
            id: "approval-r2".into(),
            risk: 2,
            status: "pending".into(),
        }
    }

    #[test]
    fn visible_r2_with_correct_slug_allows_approve() {
        let approval = pending_r2();
        let slug = spoken_slug(&approval.id);
        let decision = voice_approval_decision(
            &approval,
            &PopupState {
                visible_approval_id: Some(approval.id.clone()),
            },
            &format!("approve {slug}"),
            true,
        );
        assert_eq!(
            decision,
            VoiceApprovalDecision::Allow {
                approval_id: approval.id,
                status: "approved".into()
            }
        );
    }

    #[test]
    fn visible_r2_with_correct_slug_allows_deny() {
        let approval = pending_r2();
        let slug = spoken_slug(&approval.id);
        let decision = voice_approval_decision(
            &approval,
            &PopupState {
                visible_approval_id: Some(approval.id.clone()),
            },
            &format!("deny {slug}"),
            false,
        );
        assert_eq!(
            decision,
            VoiceApprovalDecision::Allow {
                approval_id: approval.id,
                status: "denied".into()
            }
        );
    }

    #[test]
    fn hidden_popup_refuses() {
        let approval = pending_r2();
        let slug = spoken_slug(&approval.id);
        assert!(matches!(
            voice_approval_decision(
                &approval,
                &PopupState {
                    visible_approval_id: None
                },
                &format!("approve {slug}"),
                true,
            ),
            VoiceApprovalDecision::Refuse { .. }
        ));
    }

    #[test]
    fn r3_refuses_even_with_slug() {
        let approval = ApprovalSnapshot {
            id: "approval-r3".into(),
            risk: 3,
            status: "pending".into(),
        };
        let slug = spoken_slug(&approval.id);
        assert!(matches!(
            voice_approval_decision(
                &approval,
                &PopupState {
                    visible_approval_id: Some(approval.id.clone())
                },
                &format!("approve {slug}"),
                true,
            ),
            VoiceApprovalDecision::Refuse { .. }
        ));
    }

    #[test]
    fn wrong_slug_refuses() {
        let approval = pending_r2();
        assert!(matches!(
            voice_approval_decision(
                &approval,
                &PopupState {
                    visible_approval_id: Some(approval.id.clone())
                },
                "approve wrong-slug",
                true,
            ),
            VoiceApprovalDecision::Refuse { .. }
        ));
    }

    #[test]
    fn non_pending_refuses() {
        let approval = ApprovalSnapshot {
            id: "approval-old".into(),
            risk: 2,
            status: "approved".into(),
        };
        let slug = spoken_slug(&approval.id);
        assert!(matches!(
            voice_approval_decision(
                &approval,
                &PopupState {
                    visible_approval_id: Some(approval.id.clone())
                },
                &format!("approve {slug}"),
                true,
            ),
            VoiceApprovalDecision::Refuse { .. }
        ));
    }
}
