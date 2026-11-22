use thiserror::Error;

use crate::agent_access::error::AgentAccessError;
use crate::agent_identity::error::AgentIdentityError;
use crate::identity::AgentIdentityFailure;

// LiteLLM's raise_identity_failure turns a failure into a status and a string detail; the caller writes the HTTP answer
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{detail}")]
pub struct AdmissionError {
    pub status: u16,
    pub detail: String,
}

impl From<AgentIdentityFailure> for AdmissionError {
    fn from(failure: AgentIdentityFailure) -> Self {
        let converted = AgentIdentityError::from(failure);
        Self {
            status: converted.status,
            detail: converted.message,
        }
    }
}

impl From<AgentAccessError> for AdmissionError {
    fn from(error: AgentAccessError) -> Self {
        Self {
            status: error.status(),
            detail: error.to_string(),
        }
    }
}
