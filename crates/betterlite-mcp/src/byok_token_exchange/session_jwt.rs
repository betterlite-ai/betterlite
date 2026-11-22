mod tests;

use std::time::{Duration, TryFromFloatSecsError};

use jsonwebtoken::errors::Error as JwtError;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SESSION_TOKEN_TYPE: &str = "byok_session";
pub const SESSION_TOKEN_TTL_SECONDS: u64 = 3600;

// the payload order is the one PyJWT writes for the dict LiteLLM builds
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionClaims {
    pub user_id: String,
    pub server_id: String,
    #[serde(rename = "type")]
    pub token_type: String,
    pub iat: u64,
    pub exp: u64,
}

// both payloads are boxed so neither variant is over 3x the other, the limit variant_size_differences enforces
#[derive(Debug, Error)]
pub enum SessionTokenError {
    #[error("the session token issue time is not a Unix timestamp")]
    IssuedAt(#[source] Box<TryFromFloatSecsError>),
    #[error("the session token could not be signed")]
    Encode(#[source] Box<JwtError>),
}

pub fn issue_session_token(
    user_id: &str,
    server_id: &str,
    master_key: &SecretString,
    iat: f64,
) -> Result<String, SessionTokenError> {
    let issued_at = Duration::try_from_secs_f64(iat)
        .map_err(|source| SessionTokenError::IssuedAt(Box::new(source)))?
        .as_secs();
    let claims = SessionClaims {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
        token_type: SESSION_TOKEN_TYPE.to_owned(),
        iat: issued_at,
        exp: issued_at.saturating_add(SESSION_TOKEN_TTL_SECONDS),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(master_key.expose_secret().as_bytes()),
    )
    .map_err(|source| SessionTokenError::Encode(Box::new(source)))
}
