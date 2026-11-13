use betterlite_http::error::HttpError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillsError {
    #[error("ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN is required for Skills API")]
    MissingCredential,
    #[error("the {name} header value is not a valid HTTP header value")]
    InvalidHeader { name: &'static str },
    #[error("the skills response body is not valid for its type: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Url(#[from] HttpError),
}
