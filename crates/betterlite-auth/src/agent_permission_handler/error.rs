use betterlite_db::postgres_store::errors::DbError;

use crate::agent_access_groups::error::AccessGroupsError;
use crate::agent_caller::error::AgentCallerError;
use crate::auth_model_patterns::access_groups::AccessGroupError;
use crate::auth_team_lookup::error::TeamLookupError;

// LiteLLM's strict-mode HTTPException(503, "Agent invocation policy is unavailable") and the read error it carries
#[derive(Debug, thiserror::Error)]
pub enum GrantError {
    #[error("Agent invocation policy is unavailable")]
    PolicyUnavailable(#[source] Box<Self>),
    #[error("agent grant lookup failed: {0}")]
    Lookup(String),
}

impl GrantError {
    // the HTTP status LiteLLM sends for this failure; None is a read error that only non-strict callers log
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::PolicyUnavailable(_) => Some(503),
            Self::Lookup(_) => None,
        }
    }
}

impl From<TeamLookupError> for GrantError {
    fn from(source: TeamLookupError) -> Self {
        Self::Lookup(source.to_string())
    }
}

impl From<AccessGroupsError> for GrantError {
    fn from(source: AccessGroupsError) -> Self {
        Self::Lookup(source.to_string())
    }
}

impl From<AccessGroupError> for GrantError {
    fn from(source: AccessGroupError) -> Self {
        Self::Lookup(source.to_string())
    }
}

impl From<AgentCallerError> for GrantError {
    fn from(source: AgentCallerError) -> Self {
        Self::Lookup(source.to_string())
    }
}

impl From<DbError> for GrantError {
    fn from(source: DbError) -> Self {
        Self::Lookup(source.to_string())
    }
}
