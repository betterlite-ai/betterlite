use betterlite_prompts::cohere::error::CohereError as HistoryError;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::error::{ErrorRequest, ProviderError, ProviderErrorParts};
use crate::openai_base_config::Headers;

const REQUEST_METHOD: &str = "POST";
// LiteLLM's v1 CohereError is not the common_utils class: it carries a synthetic v1 chat request
const REQUEST_URL: &str = "https://api.cohere.ai/v1/chat";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct CohereError {
    pub status_code: u16,
    pub message: String,
}

impl From<CohereError> for ProviderError {
    fn from(error: CohereError) -> Self {
        // the response repeats the same status and request, as httpx.Response does in LiteLLM
        Self::from(ProviderErrorParts {
            status_code: error.status_code,
            message: error.message,
            headers: None,
            request: Some(ErrorRequest {
                method: REQUEST_METHOD.to_owned(),
                url: REQUEST_URL.to_owned(),
            }),
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

// LiteLLM's get_error_class builds this error and never forwards the headers it was given
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, _headers: &Headers) -> CohereError {
    CohereError {
        status_code,
        message: message.to_owned(),
    }
}

#[derive(Debug, Error)]
pub enum CohereV1ConfigError {
    #[error("Cohere v1 chat has no {key} setting")]
    UnknownSetting { key: String },
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error("Cohere v1 tools is not a list")]
    ToolsNotList,
    #[error("Cohere v1 tool {index} cannot be translated: {detail}")]
    Tool { index: usize, detail: &'static str },
    #[error("Cohere v1 tool {index} is not an OpenAI function definition: {source}")]
    ToolShape {
        index: usize,
        #[source]
        source: JsonError,
    },
}
