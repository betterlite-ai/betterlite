use reqwest::StatusCode;
use thiserror::Error;

use super::wire::LakeraResponse;

#[derive(Debug, Error)]
pub enum LakeraPiiError {
    #[error("Violated guardrail policy")]
    Blocked { response: Option<LakeraResponse> },
}

impl LakeraPiiError {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Blocked { .. } => StatusCode::BAD_REQUEST,
        }
    }

    #[must_use]
    pub const fn response(&self) -> Option<&LakeraResponse> {
        match self {
            Self::Blocked { response } => response.as_ref(),
        }
    }
}
