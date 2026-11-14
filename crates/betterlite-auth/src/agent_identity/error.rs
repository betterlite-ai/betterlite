use thiserror::Error;

use crate::identity::{AgentIdentityFailure, FailureCode};

pub const IDENTITY_DENIED_STATUS: u16 = 403;
pub const POLICY_UNAVAILABLE_STATUS: u16 = 503;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{message}")]
pub struct AgentIdentityError {
    pub status: u16,
    pub message: String,
}

#[must_use]
pub fn raise_identity_failure(
    failure: AgentIdentityFailure,
    status_code: u16,
) -> AgentIdentityError {
    AgentIdentityError {
        status: match &failure.code {
            FailureCode::PolicyUnavailable => POLICY_UNAVAILABLE_STATUS,
            FailureCode::IdentityDenied => status_code,
        },
        message: failure.message,
    }
}

impl From<AgentIdentityFailure> for AgentIdentityError {
    fn from(failure: AgentIdentityFailure) -> Self {
        raise_identity_failure(failure, IDENTITY_DENIED_STATUS)
    }
}
