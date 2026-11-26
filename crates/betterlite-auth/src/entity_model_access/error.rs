use betterlite_db::team_rows::decoder::TeamRowError;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;

use crate::auth_model_patterns::denial::ModelAccessDecision;
use crate::auth_org_lookup::error::OrgLookupError;
use crate::team_budget_admission::error::TeamBudgetError;

// Everything the wrappers raise: B747's denial, its bare Exception, and the entity reads behind them.
#[derive(Debug, thiserror::Error)]
pub enum EntityModelAccessError {
    #[error("the model grant denies the requested model")]
    // ast-grep-ignore: no-long-names
    Denied(Box<ModelAccessDeniedProxyException>),
    // _can_object_call_model raises a plain Exception here, which only the outer catches skip
    #[error("Unable to parse model, max fallback depth exceeded - received model: {model}")]
    RecurseDepth { model: String },
    #[error(transparent)]
    Member(Box<TeamBudgetError>),
    #[error(transparent)]
    Project(Box<OrgLookupError>),
    #[error("the fallback team object could not be built")]
    TeamRow(#[source] Box<TeamRowError>),
}

impl EntityModelAccessError {
    #[must_use]
    pub fn gate(decision: ModelAccessDecision) -> Result<(), Self> {
        match decision {
            ModelAccessDecision::Allowed => Ok(()),
            ModelAccessDecision::Denied(denial) => Err(Self::Denied(denial)),
            ModelAccessDecision::RecurseDepthExceeded { model } => {
                Err(Self::RecurseDepth { model })
            }
        }
    }

    // the member scope's denial keeps B747's client message and its team type
    #[must_use]
    pub fn from_member(error: TeamBudgetError) -> Self {
        match error {
            TeamBudgetError::ModelDenied(denial) => Self::Denied(denial),
            TeamBudgetError::RecurseDepth { model } => Self::RecurseDepth { model },
            other @ (TeamBudgetError::Denied(_)
            | TeamBudgetError::Spend(_)
            | TeamBudgetError::Membership(_)
            | TeamBudgetError::DefaultBudget(_)) => Self::Member(Box::new(other)),
        }
    }

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn denial(&self) -> Option<&ModelAccessDeniedProxyException> {
        match self {
            Self::Denied(denial) => Some(denial),
            Self::RecurseDepth { .. } | Self::Member(_) | Self::Project(_) | Self::TeamRow(_) => {
                None
            }
        }
    }
}
