use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RealtimeError {
    #[error("api_key is required for OpenAI realtime calls")]
    MissingApiKey,
    #[error("api_base is not a valid URL: {source}")]
    InvalidApiBase {
        #[source]
        source: url::ParseError,
    },
    // LiteLLM's pre_call runs inside the connection try, so its failure takes the generic 1011 close path
    #[error("the realtime pre-call logging failed")]
    Logging {
        #[source]
        source: PrePostError,
    },
    #[error("Unexpected error while closing WebSocket: {close_error}")]
    UnexpectedCloseFailure { close_error: String },
}
