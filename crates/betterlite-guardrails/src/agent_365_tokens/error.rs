use thiserror::Error;

use crate::agent_365_tokens::poster::Agent365PosterError;
use crate::guardrail_base::error::GuardrailBaseError;

pub const GATEWAY_OWNED_TOKEN_ERRORS: [&str; 4] = [
    "invalid_client",
    "unauthorized_client",
    "invalid_scope",
    "invalid_resource",
];
// Entra reports a malformed or unverifiable assertion as invalid_client too; only its AADSTS50027xx sub-codes tell that apart from a bad gateway secret.
pub const ASSERTION_ERROR_AADSTS_PREFIX: &str = "50027";
pub const MALFORMED_NON_JSON_BODY: &str = "the Entra token endpoint returned a non-JSON body";
pub const MALFORMED_NON_OBJECT_BODY: &str =
    "the Entra token endpoint returned a non-object JSON body";
pub const MALFORMED_MISSING_ACCESS_TOKEN: &str =
    "the Entra token endpoint returned no access_token";
pub const MALFORMED_BAD_ACCESS_TOKEN: &str =
    "the Entra token endpoint returned a non-string access_token";

#[derive(Debug, Error)]
pub enum Agent365Error {
    #[error(transparent)]
    Guardrail(#[from] GuardrailBaseError),
    #[error("Agent 365 request_timeout must be a non-negative number of seconds, got {seconds}")]
    Timeout { seconds: f64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{error_code}: {description}")]
pub struct Agent365TokenExchangeError {
    pub status_code: u16,
    pub error_code: String,
    pub description: String,
    pub aadsts_codes: Vec<i64>,
}

impl Agent365TokenExchangeError {
    #[must_use]
    pub fn gateway_owned(&self) -> bool {
        GATEWAY_OWNED_TOKEN_ERRORS.contains(&self.error_code.as_str())
            && !self
                .aadsts_codes
                .iter()
                .any(|code| code.to_string().starts_with(ASSERTION_ERROR_AADSTS_PREFIX))
    }
}

#[derive(Debug, Error)]
pub enum Agent365TokenError {
    #[error(transparent)]
    Exchange(#[from] Agent365TokenExchangeError),
    #[error("{message}")]
    Malformed { message: &'static str },
    #[error("HTTP {status}")]
    Throttled { status: u16 },
    #[error("HTTP {status}")]
    HttpStatus { status: u16 },
    #[error(transparent)]
    Poster(#[from] Agent365PosterError),
}
