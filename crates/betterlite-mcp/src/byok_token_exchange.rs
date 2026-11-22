pub mod exchange;
pub mod pkce;
pub mod ports;
pub mod session_jwt;

use http::StatusCode;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenForm {
    pub grant_type: String,
    pub code: String,
    pub redirect_uri: String,
    pub code_verifier: String,
    pub client_id: String,
}

// the response carries a live access token, so it derives no Debug
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct TokenGrant {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TokenErrorBody {
    pub error: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TokenError {
    #[error("unsupported_grant_type")]
    UnsupportedGrantType,
    #[error("invalid_grant")]
    InvalidGrant,
    #[error("server_error")]
    ServerError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenErrorResponse {
    pub status: StatusCode,
    pub body: TokenErrorBody,
}

impl TokenError {
    #[must_use]
    pub const fn response(self) -> TokenErrorResponse {
        let (status, error) = match self {
            Self::UnsupportedGrantType => (StatusCode::BAD_REQUEST, "unsupported_grant_type"),
            Self::InvalidGrant => (StatusCode::BAD_REQUEST, "invalid_grant"),
            Self::ServerError => (StatusCode::INTERNAL_SERVER_ERROR, "server_error"),
        };
        TokenErrorResponse {
            status,
            body: TokenErrorBody { error },
        }
    }
}
