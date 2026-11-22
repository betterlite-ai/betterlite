use thiserror::Error;

use super::handler::HandlerError;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Handler(#[from] HandlerError),
    // the class config and the request disagree on the shape the extra_body merge works on
    #[error("extra_body must be an object to merge the {provider} config into")]
    ExtraBody { provider: &'static str },
    #[error("the post_call record could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl DispatchError {
    // the status LiteLLM's provider exception carries for this failure
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Handler(error) => error.status_code(),
            Self::ExtraBody { .. } | Self::Encode { .. } => None,
        }
    }
}
