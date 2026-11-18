use axum::http::StatusCode;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DelegationError {
    #[error("Non-admin users cannot remove the user_id from a key.")]
    RemoveUserId,
    #[error(
        "Non-admin caller is not allowed to rebind the key from user={existing} to user={incoming}"
    )]
    RebindUserId { existing: String, incoming: String },
    #[error(
        "Only proxy admins can set `allowed_routes` on a key. Use `key_type` to pick a preset route bucket instead."
    )]
    AllowedRoutes,
    #[error("Only proxy admins can set `permissions`.")]
    Permissions,
    #[error("budget_limits entry max_budget ({amount}) must be a finite number.")]
    NonFiniteBudget { amount: String },
    #[error(
        "budget_limits cannot be set without specifying team_id when using a CLI session token."
    )]
    SessionTokenWithoutTeam,
    #[error(
        "budget_limits entry max_budget ({amount}) cannot exceed the caller's own max_budget ({ceiling})."
    )]
    AboveCeiling { amount: String, ceiling: String },
}

// LiteLLM hands HTTPException either a string detail or an {"error": ...} object, and callers keep that shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegationDetail {
    Message(String),
    Error(String),
}

impl DelegationError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::RemoveUserId
            | Self::RebindUserId { .. }
            | Self::AllowedRoutes
            | Self::Permissions => StatusCode::FORBIDDEN,
            Self::NonFiniteBudget { .. }
            | Self::SessionTokenWithoutTeam
            | Self::AboveCeiling { .. } => StatusCode::BAD_REQUEST,
        }
    }

    #[must_use]
    pub fn detail(&self) -> DelegationDetail {
        match self {
            Self::RemoveUserId | Self::RebindUserId { .. } => {
                DelegationDetail::Message(self.to_string())
            }
            Self::AllowedRoutes
            | Self::Permissions
            | Self::NonFiniteBudget { .. }
            | Self::SessionTokenWithoutTeam
            | Self::AboveCeiling { .. } => DelegationDetail::Error(self.to_string()),
        }
    }
}
