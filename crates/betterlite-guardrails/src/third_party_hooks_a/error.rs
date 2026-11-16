use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use thiserror::Error;

use super::lakera::error::LakeraHookError;
use crate::aim::error::AimError;
use crate::akto::error::AktoError;
use crate::conduct::error::ConductCheckError;
use crate::lasso::error::LassoError;

// What a hook body hands back to the proxy: an HTTP rejection or a plain failure, plus the class
// errors of the five vendors passed on unchanged so their owner can map them.
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
    // The proxy picks the HTTP status from the variant alone, never from the message text.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Rejection { status, .. } => *status,
            Self::Failure { .. } | Self::ConductMissingPackage(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Aim(error) => match error {
                AimError::Rejected { .. } => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            Self::Akto(error) => error.status_code(),
            Self::Lakera(error) => error.status_code(),
            Self::Lasso(error) => match error {
                LassoError::Blocked { .. } => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
        }
    }
}
