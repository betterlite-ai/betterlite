use betterlite_tokenizer::count::Counter;

use crate::a2a_text::A2aResponse;
use crate::a2a_text::request_utils::{A2aRequest, usage_from_request_response};
use crate::client::records::compat::SendMessageRequest;
use crate::client::records::response::SendMessageResponse;

use super::error::SendCallError;

// LiteLLM's asyncified A2ARequestUtils.calculate_usage_from_request_response, reading the response's root dump
pub(super) async fn usage_from(
    counter: &Counter,
    request: &SendMessageRequest,
    response: &SendMessageResponse,
) -> Result<(u64, u64), SendCallError> {
    let request_json = serde_json::to_value(request).map_err(|source| SendCallError::Json {
        value: "request",
        source,
    })?;
    let response_json = serde_json::to_value(response).map_err(|source| SendCallError::Json {
        value: "response",
        source,
    })?;
    let a2a_request = serde_json::from_value::<A2aRequest>(request_json).map_err(|source| {
        SendCallError::Json {
            value: "request",
            source,
        }
    })?;
    let a2a_response = serde_json::from_value::<A2aResponse>(response_json).map_err(|source| {
        SendCallError::Json {
            value: "response",
            source,
        }
    })?;
    let (prompt_tokens, completion_tokens, _) =
        usage_from_request_response(counter, &a2a_request, &a2a_response).await?;
    let prompt_tokens_u64 = u64::try_from(prompt_tokens).map_err(SendCallError::TokenCount)?;
    let completion_tokens_u64 =
        u64::try_from(completion_tokens).map_err(SendCallError::TokenCount)?;
    Ok((prompt_tokens_u64, completion_tokens_u64))
}
