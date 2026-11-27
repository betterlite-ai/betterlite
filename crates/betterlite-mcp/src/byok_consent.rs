pub mod get;
mod location;
pub mod post;
pub mod session;
pub mod store;

#[path = "byok_consent/auth_code_limit/tests.rs"]
mod auth_code_limit;
#[path = "byok_consent/auth_code_ttl/tests.rs"]
mod auth_code_ttl;
#[path = "byok_consent/escaping/tests.rs"]
mod escaping;
#[path = "byok_consent/form_structure/tests.rs"]
mod form_structure;
#[path = "byok_consent/session_auth/tests.rs"]
mod session_auth;
#[path = "byok_consent/session_liveness/tests.rs"]
mod session_liveness;
#[cfg(test)]
#[path = "byok_consent/tests/support.rs"]
pub(crate) mod test_support;

use http::StatusCode;
use serde::Serialize;
use thiserror::Error;

use crate::redirect_trust::shape::{RedirectUriRejected, RejectionDetail};
use crate::session_admission::auth::LOGIN_REQUIRED;

#[derive(Debug, Error)]
pub enum ConsentError {
    #[error("login_required")]
    LoginRequired,
    #[error("response_type must be 'code'")]
    ResponseType,
    #[error("redirect_uri is required")]
    RedirectUriRequired,
    #[error("code_challenge is required")]
    CodeChallengeRequired,
    #[error("Only S256 code_challenge_method is supported")]
    UnsupportedChallengeMethod,
    #[error("Too many pending authorization flows")]
    TooManyPendingFlows,
    #[error(transparent)]
    RedirectUri(#[from] RedirectUriRejected),
}

impl ConsentError {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::LoginRequired => StatusCode::UNAUTHORIZED,
            Self::TooManyPendingFlows => StatusCode::SERVICE_UNAVAILABLE,
            Self::ResponseType
            | Self::RedirectUriRequired
            | Self::CodeChallengeRequired
            | Self::UnsupportedChallengeMethod
            | Self::RedirectUri(_) => StatusCode::BAD_REQUEST,
        }
    }

    #[must_use]
    pub fn body(&self) -> ConsentErrorBody<'_> {
        let detail = match self {
            Self::RedirectUri(rejection) => ConsentDetail::Rejection(rejection.detail()),
            Self::LoginRequired => ConsentDetail::Message(LOGIN_REQUIRED),
            Self::ResponseType => ConsentDetail::Message("response_type must be 'code'"),
            Self::RedirectUriRequired => ConsentDetail::Message("redirect_uri is required"),
            Self::CodeChallengeRequired => ConsentDetail::Message("code_challenge is required"),
            Self::UnsupportedChallengeMethod => {
                ConsentDetail::Message("Only S256 code_challenge_method is supported")
            }
            Self::TooManyPendingFlows => {
                ConsentDetail::Message("Too many pending authorization flows")
            }
        };
        ConsentErrorBody { detail }
    }
}

// FastAPI writes HTTPException's detail under a "detail" key, whatever the detail's shape
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConsentErrorBody<'text> {
    pub detail: ConsentDetail<'text>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ConsentDetail<'text> {
    Message(&'static str),
    Rejection(RejectionDetail<'text>),
}
