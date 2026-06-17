/// All actions the RATO agent can propose.
///
/// # Hard Invariant (§11, binding)
/// The tier table is fixed in code. There is no public API that mutates tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    // R0 — read-only / autonomous
    SensorRead,
    TranscriptParse,

    // R1 — reversible managed writes (autonomous + audit)
    WorktreeCreate,
    WorkbenchFileWrite,
    Pin,
    DotfileEditManaged,
    McpKnownEdit,

    // R2 — operator-visible side effects (approval required)
    CommandOutsideWorktree,
    TerminalInject,
    MergeBack,
    LiveRepoWrite,
    ProjectLocalInstall,
    McpNewBinary,

    // R3 — system-level / hard-to-reverse (approval + typed-slug confirmation)
    GlobalInstall,
    ShellStartupEdit,
    ConfigOutsideSafe,
    GitForcePush,

    // Refused — never proposable
    SudoAnything,
}

/// Risk tiers for proposable actions (R0 through R3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    R0,
    R1,
    R2,
    R3,
}

/// Outcome of a risk classification.
///
/// `SudoAnything` maps to `Refused` and must never reach the approval path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskOutcome {
    Tier(Tier),
    Refused,
}

/// Map an `ActionKind` to its `RiskOutcome`.
///
/// The table is fixed in code — there are no setters or config loading.
pub fn risk_tier(kind: ActionKind) -> RiskOutcome {
    match kind {
        ActionKind::SensorRead | ActionKind::TranscriptParse => RiskOutcome::Tier(Tier::R0),

        ActionKind::WorktreeCreate
        | ActionKind::WorkbenchFileWrite
        | ActionKind::Pin
        | ActionKind::DotfileEditManaged
        | ActionKind::McpKnownEdit => RiskOutcome::Tier(Tier::R1),

        ActionKind::CommandOutsideWorktree
        | ActionKind::TerminalInject
        | ActionKind::MergeBack
        | ActionKind::LiveRepoWrite
        | ActionKind::ProjectLocalInstall
        | ActionKind::McpNewBinary => RiskOutcome::Tier(Tier::R2),

        ActionKind::GlobalInstall
        | ActionKind::ShellStartupEdit
        | ActionKind::ConfigOutsideSafe
        | ActionKind::GitForcePush => RiskOutcome::Tier(Tier::R3),

        ActionKind::SudoAnything => RiskOutcome::Refused,
    }
}

/// Returns `true` for R2 and R3 — the operator must explicitly approve the action.
///
/// # Panics
/// Does not panic. Only call with a `Tier` (not `Refused`); use `risk_tier` first.
pub fn requires_approval(tier: Tier) -> bool {
    matches!(tier, Tier::R2 | Tier::R3)
}

/// Returns `true` for R3 — the operator must type a confirmation slug in addition to approving.
///
/// # Panics
/// Does not panic.
pub fn requires_slug(tier: Tier) -> bool {
    matches!(tier, Tier::R3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table-driven coverage: every `ActionKind` variant → expected `RiskOutcome`.
    #[test]
    fn risk_tier_table() {
        let cases: &[(ActionKind, RiskOutcome)] = &[
            // R0
            (ActionKind::SensorRead, RiskOutcome::Tier(Tier::R0)),
            (ActionKind::TranscriptParse, RiskOutcome::Tier(Tier::R0)),
            // R1
            (ActionKind::WorktreeCreate, RiskOutcome::Tier(Tier::R1)),
            (ActionKind::WorkbenchFileWrite, RiskOutcome::Tier(Tier::R1)),
            (ActionKind::Pin, RiskOutcome::Tier(Tier::R1)),
            (ActionKind::DotfileEditManaged, RiskOutcome::Tier(Tier::R1)),
            (ActionKind::McpKnownEdit, RiskOutcome::Tier(Tier::R1)),
            // R2
            (
                ActionKind::CommandOutsideWorktree,
                RiskOutcome::Tier(Tier::R2),
            ),
            (ActionKind::TerminalInject, RiskOutcome::Tier(Tier::R2)),
            (ActionKind::MergeBack, RiskOutcome::Tier(Tier::R2)),
            (ActionKind::LiveRepoWrite, RiskOutcome::Tier(Tier::R2)),
            (ActionKind::ProjectLocalInstall, RiskOutcome::Tier(Tier::R2)),
            (ActionKind::McpNewBinary, RiskOutcome::Tier(Tier::R2)),
            // R3
            (ActionKind::GlobalInstall, RiskOutcome::Tier(Tier::R3)),
            (ActionKind::ShellStartupEdit, RiskOutcome::Tier(Tier::R3)),
            (ActionKind::ConfigOutsideSafe, RiskOutcome::Tier(Tier::R3)),
            (ActionKind::GitForcePush, RiskOutcome::Tier(Tier::R3)),
            // Refused — must NOT map to any tier
            (ActionKind::SudoAnything, RiskOutcome::Refused),
        ];

        for (kind, expected) in cases {
            let actual = risk_tier(*kind);
            assert_eq!(
                actual, *expected,
                "risk_tier({kind:?}) = {actual:?}, expected {expected:?}"
            );
        }
    }

    /// `requires_approval` is false for R0/R1, true for R2/R3.
    #[test]
    fn approval_flags() {
        assert!(!requires_approval(Tier::R0));
        assert!(!requires_approval(Tier::R1));
        assert!(requires_approval(Tier::R2));
        assert!(requires_approval(Tier::R3));
    }

    /// `requires_slug` is only true for R3.
    #[test]
    fn slug_flags() {
        assert!(!requires_slug(Tier::R0));
        assert!(!requires_slug(Tier::R1));
        assert!(!requires_slug(Tier::R2));
        assert!(requires_slug(Tier::R3));
    }

    /// `SudoAnything` must be `Refused`, not any tier variant.
    #[test]
    fn sudo_is_refused_not_a_tier() {
        let outcome = risk_tier(ActionKind::SudoAnything);
        assert_eq!(outcome, RiskOutcome::Refused);
        // Ensure it is not mistakenly R3 or any other tier
        assert!(
            !matches!(outcome, RiskOutcome::Tier(_)),
            "SudoAnything must never resolve to a tier"
        );
    }
}
