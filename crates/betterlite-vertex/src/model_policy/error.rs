use std::collections::BTreeMap;

use betterlite_error::error::ErrorClass;
use betterlite_error::facts::ProviderErrorFacts;
use betterlite_error::vertex_errors::map_vertex_exception;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct VertexAiError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<BTreeMap<String, String>>,
}

impl From<&VertexAiError> for ProviderErrorFacts {
    fn from(error: &VertexAiError) -> Self {
        Self {
            status_code: Some(error.status_code),
            status_code_is_synthesized: false,
            message: Some(error.message.clone()),
            body: None,
            headers: error.headers.clone(),
            response: None,
            litellm_response_headers: None,
            request: None,
        }
    }
}

#[must_use]
pub fn classify_vertex_error(
    error: &VertexAiError,
    custom_llm_provider: &str,
    model: Option<String>,
) -> Option<ErrorClass> {
    map_vertex_exception(
        &ProviderErrorFacts::from(error),
        &error.message,
        custom_llm_provider,
        model,
    )
}
