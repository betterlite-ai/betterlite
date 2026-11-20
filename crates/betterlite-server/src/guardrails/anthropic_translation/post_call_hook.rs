#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod error;
mod tests;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::messages_chat_output::records::ResponseRecord;
use betterlite_core::api::messages_chat_output::response::response_message;

use error::PostCallHookError;

const CHOICES: &str = "choices";

// The legacy post-call hook reads an OpenAI ModelResponse, so the Anthropic shape is handed to it translated.
pub fn post_call_hook_response(
    response: &GuardrailValue,
) -> Result<GuardrailValue, PostCallHookError> {
    let GuardrailValue::Object(fields) = response else {
        return Ok(response.clone());
    };
    // A JSON value has no class, and an Anthropic message never has choices.
    if !matches!(fields.get(CHOICES), Some(GuardrailValue::Array(_))) {
        return Ok(response.clone());
    }
    let request_json =
        serde_json::to_string(response).map_err(|source| PostCallHookError::Response { source })?;
    let record: ResponseRecord = serde_json::from_str(&request_json)
        .map_err(|source| PostCallHookError::Response { source })?;
    let message = response_message(&record, None, None)?;
    let message_json =
        serde_json::to_string(&message).map_err(|source| PostCallHookError::Encode { source })?;
    serde_json::from_str(&message_json).map_err(|source| PostCallHookError::Encode { source })
}
