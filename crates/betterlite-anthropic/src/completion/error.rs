use std::str::Utf8Error;

use betterlite_openai::error::{
    ErrorRequest, ErrorResponse, OpenAiError, ProviderError, ProviderErrorParts,
};
use betterlite_prompts::error::FactoryError;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::selection::SelectionError;
use thiserror::Error;

use crate::option_primitives::OptionPrimitivesError;

pub const COMPLETE_URL: &str = "https://api.anthropic.com/v1/complete";

#[derive(Debug, Error)]
pub enum CompletionError {
    #[error(
        "Missing Anthropic API Key - A call is being made to anthropic but no key is set either in the environment variables or via params"
    )]
    MissingApiKey,
    #[error("the Anthropic API key is not a valid header value")]
    InvalidApiKey,
    #[error("{}", .0.message)]
    Upstream(Box<ProviderError>),
    #[error(transparent)]
    Options(Box<OptionPrimitivesError>),
    #[error(transparent)]
    Config(Box<OpenAiError>),
    #[error("an option default does not convert to JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Prompt(Box<FactoryError>),
    #[error("the prompt for the legacy completion is not text")]
    PromptNotText,
    #[error("the completion reply has no {field} field")]
    MissingField { field: &'static str },
    #[error("the completion reply field {field} is not {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the model response has no choices")]
    NoChoices,
    #[error("the stream chunk is not a JSON object")]
    ChunkNotObject,
    #[error("the token count does not fit the usage block")]
    Usage,
    #[error("the text could not be tokenized")]
    Tokenize {
        #[source]
        source: Box<EngineError>,
    },
    #[error("the default encoding could not be loaded")]
    DefaultEncoding {
        #[source]
        source: Box<SelectionError>,
    },
    #[error("the completion reply is not valid UTF-8")]
    NotUtf8 {
        #[source]
        source: Utf8Error,
    },
}

// AnthropicTextError always carries the fixed /v1/complete request and a response of the same status
#[must_use]
pub fn anthropic_text_error(status_code: u16, message: String) -> ProviderError {
    let request = ErrorRequest {
        method: "POST".to_owned(),
        url: COMPLETE_URL.to_owned(),
    };
    ProviderError::from(ProviderErrorParts {
        status_code,
        message,
        headers: None,
        response: Some(ErrorResponse {
            status_code,
            request: request.clone(),
        }),
        request: Some(request),
        body: None,
        status_code_is_synthesized: false,
    })
}
