use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException; // ast-grep-ignore: no-long-names

use crate::agent_access_groups::error::AccessGroupsError;
use crate::agent_caller::error::AgentCallerError;
use crate::auth_model_patterns::denial::ModelAccessDecision;
use crate::entity_model_access::error::EntityModelAccessError;

// Everything the agent grant checks raise: the denials, the reads behind them and a missing registry.
#[derive(Debug, thiserror::Error)]
pub enum AgentModelGrantsError {
    // an agent key reached the check before the state installed its access source
    #[error("the agent access source is not installed")]
    Unavailable,
    #[error("the model grant denies the requested model")]
    // ast-grep-ignore: no-long-names
    Denied(Box<ModelAccessDeniedProxyException>),
    #[error("Unable to parse model, max fallback depth exceeded - received model: {model}")]
    RecurseDepth { model: String },
    #[error(transparent)]
    Ceiling(Box<AccessGroupsError>),
    #[error(transparent)]
    Caller(Box<AgentCallerError>),
    #[error(transparent)]
    Entity(Box<EntityModelAccessError>),
}

impl AgentModelGrantsError {
    pub fn gate(decision: ModelAccessDecision) -> Result<(), Self> {
        match decision {
            ModelAccessDecision::Allowed => Ok(()),
            ModelAccessDecision::Denied(denial) => Err(Self::Denied(denial)),
            ModelAccessDecision::RecurseDepthExceeded { model } => {
                Err(Self::RecurseDepth { model })
            }
        }
    }
}

impl From<AccessGroupsError> for AgentModelGrantsError {
    fn from(error: AccessGroupsError) -> Self {
        Self::Ceiling(Box::new(error))
    }
}

impl From<AgentCallerError> for AgentModelGrantsError {
    fn from(error: AgentCallerError) -> Self {
        Self::Caller(Box::new(error))
    }
}

impl From<EntityModelAccessError> for AgentModelGrantsError {
    fn from(error: EntityModelAccessError) -> Self {
        Self::Entity(Box::new(error))
    }
}
