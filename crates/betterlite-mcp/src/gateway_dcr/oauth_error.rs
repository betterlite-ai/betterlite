mod tests;

use serde::Serialize;
use strum::{Display, IntoStaticStr};

use crate::oauth_resource::challenge::TOKEN_NO_CACHE_HEADERS;

#[derive(Clone, Copy, Debug, Display, Eq, IntoStaticStr, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum OauthErrorCode {
    InvalidRequest,
    InvalidClient,
    InvalidGrant,
    InvalidTarget,
    InvalidRedirectUri,
    InvalidClientMetadata,
    UnsupportedResponseType,
    UnsupportedGrantType,
    AccessDenied,
    LoginRequired,
    TemporarilyUnavailable,
    ServerError,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OauthError {
    pub status: u16,
    pub error: OauthErrorCode,
    pub description: String,
}

pub trait OauthErrorResponse {
    #[must_use]
    fn body(&self) -> OauthErrorBody<'_>;

    // every RFC 6749 5.2 error response tells the client not to cache it
    #[must_use]
    fn headers(&self) -> [(&'static str, &'static str); 2] {
        TOKEN_NO_CACHE_HEADERS
    }
}

impl OauthErrorResponse for OauthError {
    fn body(&self) -> OauthErrorBody<'_> {
        OauthErrorBody {
            error: self.error,
            error_description: &self.description,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OauthErrorBody<'error> {
    pub error: OauthErrorCode,
    pub error_description: &'error str,
}
