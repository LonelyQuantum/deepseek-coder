#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Read,
    Write,
    Exec,
    Network,
    Destructive,
}

impl RiskLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Exec => "exec",
            Self::Network => "network",
            Self::Destructive => "destructive",
        }
    }

    pub const fn default_approval(self) -> ApprovalRequirement {
        match self {
            Self::Read => ApprovalRequirement::None,
            Self::Write | Self::Exec | Self::Network => ApprovalRequirement::Required,
            Self::Destructive => ApprovalRequirement::AlwaysRequired,
        }
    }
}

pub const ALL_RISK_LEVELS: [RiskLevel; 5] = [
    RiskLevel::Read,
    RiskLevel::Write,
    RiskLevel::Exec,
    RiskLevel::Network,
    RiskLevel::Destructive,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRequirement {
    None,
    Required,
    AlwaysRequired,
}

impl ApprovalRequirement {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Required => "required",
            Self::AlwaysRequired => "always_required",
        }
    }

    pub const fn is_required(self) -> bool {
        matches!(self, Self::Required | Self::AlwaysRequired)
    }

    pub const fn is_persistable(self) -> bool {
        matches!(self, Self::Required)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPersistence {
    Never,
    Session,
    Workspace,
}

impl ApprovalPersistence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Session => "session",
            Self::Workspace => "workspace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalState {
    Pending,
    Approved,
    Executing,
    Completed,
    Failed,
    Rejected,
    Canceled,
    Expired,
}

impl ApprovalState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Rejected => "rejected",
            Self::Canceled => "canceled",
            Self::Expired => "expired",
        }
    }

    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Pending, Self::Approved)
                | (Self::Pending, Self::Rejected)
                | (Self::Pending, Self::Canceled)
                | (Self::Pending, Self::Expired)
                | (Self::Approved, Self::Executing)
                | (Self::Executing, Self::Completed)
                | (Self::Executing, Self::Failed)
        )
    }

    pub fn transition_to(self, next: Self) -> Result<Self, ApprovalTransitionError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(ApprovalTransitionError {
                from: self,
                to: next,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalTransitionError {
    pub from: ApprovalState,
    pub to: ApprovalState,
}

#[cfg(test)]
mod tests {
    use super::{ApprovalRequirement, ApprovalState, RiskLevel};

    #[test]
    fn risk_levels_map_to_default_approval_requirements() {
        assert_eq!(
            RiskLevel::Read.default_approval(),
            ApprovalRequirement::None
        );
        assert_eq!(
            RiskLevel::Write.default_approval(),
            ApprovalRequirement::Required
        );
        assert_eq!(
            RiskLevel::Exec.default_approval(),
            ApprovalRequirement::Required
        );
        assert_eq!(
            RiskLevel::Network.default_approval(),
            ApprovalRequirement::Required
        );
        assert_eq!(
            RiskLevel::Destructive.default_approval(),
            ApprovalRequirement::AlwaysRequired
        );
    }

    #[test]
    fn approval_state_machine_accepts_defined_transitions() {
        assert_eq!(
            ApprovalState::Pending.transition_to(ApprovalState::Approved),
            Ok(ApprovalState::Approved)
        );
        assert_eq!(
            ApprovalState::Approved.transition_to(ApprovalState::Executing),
            Ok(ApprovalState::Executing)
        );
        assert_eq!(
            ApprovalState::Executing.transition_to(ApprovalState::Completed),
            Ok(ApprovalState::Completed)
        );
    }

    #[test]
    fn approval_state_machine_rejects_undefined_transitions() {
        let error = ApprovalState::Completed
            .transition_to(ApprovalState::Executing)
            .expect_err("completed approvals cannot go back to executing");

        assert_eq!(error.from, ApprovalState::Completed);
        assert_eq!(error.to, ApprovalState::Executing);
    }
}
