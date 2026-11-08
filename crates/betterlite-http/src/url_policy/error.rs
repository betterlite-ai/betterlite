use thiserror::Error;

use crate::user_url::error::UrlParseError;

#[derive(Debug, Error)]
pub enum UrlPolicyError {
    #[error("URL scheme is not allowed")]
    SchemeNotAllowed,
    #[error("Origin mismatch on scheme")]
    OriginScheme,
    #[error("Origin mismatch on host")]
    OriginHost,
    #[error("Origin mismatch on port")]
    OriginPort,
    #[error("the URL is longer than {limit} characters")]
    TooLong { limit: usize },
    #[error(transparent)]
    Parse(#[from] UrlParseError),
    #[error("{key} uses the {scheme} scheme, and only http and https are supported")]
    UnsupportedScheme { key: String, scheme: String },
    #[error("{key} carries a user name or password that is not UTF-8")]
    InvalidUserinfo { key: String },
}
