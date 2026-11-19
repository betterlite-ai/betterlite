use std::str;

use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::error::{OpenAiError, ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::Headers;

use super::error::CopilotEmbeddingError;

// the injected B249 post-call sink sees the raw reply before B006 decodes it, as logging_obj.post_call does
pub fn transform_embedding_response(
    raw: &[u8],
    post_call: &mut dyn FnMut(&str),
) -> Result<EmbeddingResponse, CopilotEmbeddingError> {
    if let Ok(text) = str::from_utf8(raw) {
        post_call(text);
    }
    serde_json::from_slice(raw).map_err(|source| CopilotEmbeddingError::Decode { source })
}

// LiteLLM delegates the error class to the OpenAI config, so B668's OpenAIError carries the status
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> OpenAiError {
    OpenAiError::Provider(Box::new(ProviderError::from(ProviderErrorParts {
        status_code,
        message: message.to_owned(),
        headers: Some(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        ),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })))
}
