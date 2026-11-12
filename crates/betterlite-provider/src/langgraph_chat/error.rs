use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

// LiteLLM's LangGraphError: the HTTP status and message of a LangGraph failure
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct LangGraphError {
    pub status_code: u16,
    pub message: String,
}

impl From<LangGraphError> for ProviderError {
    fn from(error: LangGraphError) -> Self {
        Self::from(ProviderErrorParts {
            status_code: error.status_code,
            message: error.message,
            headers: None,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16) -> LangGraphError {
    LangGraphError {
        status_code,
        message: message.to_owned(),
    }
}

#[derive(Debug, Error)]
pub enum LangGraphFailure {
    #[error(
        "api_base is required for LangGraph. Set it via LANGGRAPH_API_BASE env var or api_base parameter."
    )]
    MissingApiBase,
    #[error("the LangGraph option {field} has the wrong type")]
    OptionType { field: &'static str },
    #[error("the LangGraph reply is not JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the LangGraph reply is not a JSON object")]
    ReplyNotObject,
    #[error("the content of the LangGraph reply message is not text")]
    ContentType,
    #[error("the LangGraph reply cannot be written as JSON")]
    Dumps {
        #[source]
        source: serde_json::Error,
    },
    #[error("the reply has no text to count")]
    NoReplyText,
    #[error("the tokens cannot be counted: {source}")]
    TokenCount {
        #[source]
        source: CountError,
    },
    #[error("the token usage overflows")]
    UsageOverflow,
}
