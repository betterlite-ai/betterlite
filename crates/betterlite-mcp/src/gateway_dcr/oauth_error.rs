mod tests;

use serde::Serialize;
use strum::Display;

use crate::oauth_resource::challenge::TOKEN_NO_CACHE_HEADERS;

#[derive(Clone, Copy, Debug, Display, Eq, PartialEq, Serialize)]
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

impl OauthErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidClient => "invalid_client",
            Self::InvalidGrant => "invalid_grant",
            Self::InvalidTarget => "invalid_target",
            Self::InvalidRedirectUri => "invalid_redirect_uri",
            Self::InvalidClientMetadata => "invalid_client_metadata",
            Self::UnsupportedResponseType => "unsupported_response_type",
            Self::UnsupportedGrantType => "unsupported_grant_type",
            Self::AccessDenied => "access_denied",
            Self::LoginRequired => "login_required",
            Self::TemporarilyUnavailable => "temporarily_unavailable",
            Self::ServerError => "server_error",
        }
    }
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
