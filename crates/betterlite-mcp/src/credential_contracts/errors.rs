mod tests;

use std::collections::BTreeMap;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CredError {
    #[error("unauthorized: {0}")]
    Unauthorized(Unauthorized),
    #[error("misconfigured: {0}")]
    Misconfigured(String),
    #[error(
        "misconfigured: auth_type none cannot be used with credentials embedded in the \
         upstream URL; remove them from the URL and configure Basic Auth with \
         auth_type: basic and auth_value: username:password"
    )]
    UrlCredentialsNotAllowed,
    #[error("upstream unavailable: {0}")]
    UpstreamUnavailable(String),
    #[error("{0}")]
    UnsupportedMode(String),
    #[error("precondition required: {0}")]
    PreconditionRequired(String),
    #[error("not implemented: {0}")]
    NotImplemented(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{detail}")]
pub struct Unauthorized {
    pub detail: String,
    pub www_authenticate: Option<String>,
    pub body: Option<BTreeMap<String, String>>,
    pub claims: Option<String>,
}

pub type CredResult<T> = Result<T, CredError>;
