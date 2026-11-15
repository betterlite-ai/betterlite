use betterlite_prompts::message_content::{Message, convert_message_content_lists};

use crate::chat::ChatMessage;

pub mod error;

use self::error::SambaNovaChatError;

const SUPPORTED_PARAMS: [&str; 9] = [
    "max_completion_tokens",
    "max_tokens",
    "response_format",
    "stop",
    "stream",
    "stream_options",
    "temperature",
    "top_p",
    "top_k",
];
const FUNCTION_CALLING_PARAMS: [&str; 3] = ["tools", "tool_choice", "parallel_tool_calls"];

#[must_use]
pub fn supported_openai_params(supports_function_calling: bool) -> Vec<&'static str> {
    let mut params = SUPPORTED_PARAMS.to_vec();
    if supports_function_calling {
        params.extend(FUNCTION_CALLING_PARAMS);
    }
    params
}

// LiteLLM's sync and async message transforms both flatten and never call the OpenAI parent one
pub fn transform_messages(
    messages: &[ChatMessage],
) -> Result<Vec<ChatMessage>, SambaNovaChatError> {
    let mut prompt = prompt_messages(messages)?;
    let flattened = convert_message_content_lists(&mut prompt);
    chat_messages(&flattened)
}

fn prompt_messages(messages: &[ChatMessage]) -> Result<Vec<Message>, SambaNovaChatError> {
    let value =
        serde_json::to_value(messages).map_err(|source| SambaNovaChatError::Message { source })?;
    serde_json::from_value(value).map_err(|source| SambaNovaChatError::Message { source })
}

fn chat_messages(messages: &[Message]) -> Result<Vec<ChatMessage>, SambaNovaChatError> {
    let value =
        serde_json::to_value(messages).map_err(|source| SambaNovaChatError::Message { source })?;
    serde_json::from_value(value).map_err(|source| SambaNovaChatError::Message { source })
}

mod tests;
