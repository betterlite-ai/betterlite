use betterlite_anthropic::response_content::ResponseContentError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::chat_choices::ChatChoicesError;
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CopilotChatError {
    // LiteLLM turns GetAPIKeyError into AuthenticationError and keeps its message
    #[error("{source}")]
    Authentication {
        model: String,
        provider: String,
        #[source]
        source: ApiKeyError,
    },
    // a failed GitHub login is not a GetAPIKeyError, so it keeps its own status
    #[error("the Copilot login failed")]
    Login(#[source] ApiKeyError),
    #[error("the native Claude reply content could not be projected")]
    Content(#[from] ResponseContentError),
    #[error("the reply usage field {name} is not an integer")]
    Usage { name: &'static str },
    #[error("the repaired reply could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the chat reply could not be decoded")]
    Parent {
        #[source]
        source: ChatChoicesError,
        response: Box<UpstreamResponse>,
    },
}

impl CopilotChatError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Authentication { .. } => Some(401),
            Self::Login(source) => Some(source.status()),
            Self::Content(_) | Self::Usage { .. } | Self::Encode(_) | Self::Parent { .. } => None,
        }
    }
}
