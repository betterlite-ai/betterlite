use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use thiserror::Error;

use super::lakera::error::LakeraHookError;
use crate::aim::error::AimError;
use crate::akto::error::AktoError;
use crate::conduct::error::ConductCheckError;
use crate::lasso::error::LassoError;

#[derive(Debug, Error)]
pub enum ThirdPartyHookError {
    #[error("the guardrail rejected the request with status {status}")]
    Rejection {
        status: StatusCode,
        detail: GuardrailValue,
        openai_code: Option<&'static str>,
    },
    #[error("{message}")]
    Failure { message: String },
    #[error(transparent)]
    ConductMissingPackage(#[from] ConductCheckError),
    #[error(transparent)]
    Aim(#[from] AimError),
    #[error(transparent)]
    Akto(#[from] AktoError),
    #[error(transparent)]
    Lakera(#[from] LakeraHookError),
    #[error(transparent)]
    Lasso(#[from] LassoError),
}

impl ThirdPartyHookError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Rejection { status, .. } => *status,
            Self::Aim(AimError::Rejected { .. }) | Self::Lasso(LassoError::Blocked { .. }) => {
                StatusCode::BAD_REQUEST
            }
            Self::Failure { .. }
            | Self::ConductMissingPackage(_)
            | Self::Aim(_)
            | Self::Lasso(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Akto(error) => error.status_code(),
            Self::Lakera(error) => error.status_code(),
        }
    }
}
