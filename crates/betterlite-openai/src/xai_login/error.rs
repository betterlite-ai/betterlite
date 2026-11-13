use std::io;

use rand::rngs::SysError;

use crate::xai_tokens::error::XaiOauthError;

#[derive(Debug, thiserror::Error)]
pub enum XaiLoginError {
    #[error(transparent)]
    Oauth(#[from] XaiOauthError),
    #[error("the random source refused to produce the xAI OAuth PKCE verifier")]
    Entropy {
        #[source]
        source: SysError,
    },
    #[error("the xAI OAuth callback listener failed")]
    Listener {
        #[source]
        source: io::Error,
    },
    #[error("the xAI OAuth callback deadline overflowed")]
    Deadline,
    #[error("timed out waiting for the xAI OAuth callback")]
    Timeout,
    #[error("the xAI OAuth callback state did not match the pending login")]
    StateMismatch,
    #[error("xAI authorization failed: {description}")]
    Provider { description: String },
    #[error("xAI authorization returned no code")]
    MissingCode,
    #[error("could not write the xAI OAuth authorization URL")]
    Output {
        #[source]
        source: io::Error,
    },
}
