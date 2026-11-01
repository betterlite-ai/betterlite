mod tests;

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredError {
    Unauthorized(Unauthorized),
    Misconfigured(String),
    UrlCredentialsNotAllowed,
    UpstreamUnavailable(String),
    UnsupportedMode(String),
    PreconditionRequired(String),
    NotImplemented(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unauthorized {
    pub detail: String,
    pub www_authenticate: Option<String>,
    pub body: Option<BTreeMap<String, String>>,
    pub claims: Option<String>,
}

pub type CredResult<T> = Result<T, CredError>;

impl CredError {
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Unauthorized(challenge) => format!("unauthorized: {}", challenge.detail),
            Self::Misconfigured(detail) => format!("misconfigured: {detail}"),
            Self::UrlCredentialsNotAllowed => {
                "misconfigured: auth_type none cannot be used with credentials embedded in the \
                 upstream URL; remove them from the URL and configure Basic Auth with \
                 auth_type: basic and auth_value: username:password"
                    .to_owned()
            }
            Self::UpstreamUnavailable(detail) => format!("upstream unavailable: {detail}"),
            Self::UnsupportedMode(detail) => detail.clone(),
            Self::PreconditionRequired(detail) => format!("precondition required: {detail}"),
            Self::NotImplemented(detail) => format!("not implemented: {detail}"),
        }
    }
}
