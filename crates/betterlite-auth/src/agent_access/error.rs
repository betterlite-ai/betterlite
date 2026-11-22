use crate::agent_identity::error::{AgentIdentityError, POLICY_UNAVAILABLE_STATUS};
use crate::identity::AgentIdentityFailure;
use crate::resolved_key::auth::AuthError;

const TEAM_NOT_GRANTED_STATUS: u16 = 403;
// LiteLLM lets these escape as unhandled exceptions, which its proxy answers with 500
const SERVER_ERROR_STATUS: u16 = 500;

#[derive(Debug, thiserror::Error)]
pub enum AgentAccessError {
    #[error("{}", .0.message)]
    Identity(AgentIdentityFailure),
    #[error("Agent invocation policy is unavailable")]
    PolicyUnavailable,
    #[error("{detail}")]
    Rejected { status: u16, detail: String },
    #[error("the object_permission of managed agent {agent_id} does not parse")]
    ObjectPermission {
        agent_id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the stand-in key for managed agent {agent_id} could not be built")]
    Record {
        agent_id: String,
        #[source]
        source: AuthError,
    },
    #[error("Select a team that grants access to this agent using x-litellm-team-id")]
    TeamNotGranted,
}

impl AgentAccessError {
    #[must_use]
    pub fn status(&self) -> u16 {
        match self {
            Self::Identity(failure) => AgentIdentityError::from(failure.clone()).status,
            Self::PolicyUnavailable => POLICY_UNAVAILABLE_STATUS,
            Self::Rejected { status, .. } => *status,
            Self::ObjectPermission { .. } | Self::Record { .. } => SERVER_ERROR_STATUS,
            Self::TeamNotGranted => TEAM_NOT_GRANTED_STATUS,
        }
    }
}
