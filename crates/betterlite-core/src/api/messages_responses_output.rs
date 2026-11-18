use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};
use crate::api::messages_chat_output::output::MessageResponse;

use error::MessagesResponsesOutputError;
use outputs::ResponsesRequestRecord;
use records::{AnthropicRequestRecord, ResponseRecord, SettingsRecord};
use request::translate_request;
use response::translate_response;

pub mod error;
pub mod options;
pub mod outputs;
pub mod records;
pub mod request;
pub mod response;

mod tests;

// RESPONSES_INCLUDE_ENCRYPTED_REASONING: reasoning items come back encrypted so the next turn can replay them
const ENCRYPTED_REASONING_INCLUDE: &str = "reasoning.encrypted_content";

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesResponsesOutpuInput {
    #[serde(default)]
    pub request: Option<AnthropicRequestRecord>,
    #[serde(default)]
    pub response: Option<ResponseRecord>,
    // translate_request(include_encrypted_reasoning=True): absent asks the provider for encrypted reasoning
    #[serde(default)]
    pub include_encrypted_reasoning: Option<bool>,
    #[serde(default)]
    pub settings: SettingsRecord,
}

#[derive(Debug, Serialize)]
pub struct MessagesResponsesOutpuResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<ResponsesRequestRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<MessageResponse>,
}

pub fn messages_responses_output(
    input: &MessagesResponsesOutpuInput,
    limits: &ApiLimits,
) -> Result<MessagesResponsesOutpuResult, MessagesResponsesOutputError> {
    Ok(MessagesResponsesOutpuResult {
        request: match input.request.as_ref() {
            Some(request) => Some(translate_request(
                request,
                &input.settings,
                input.include_encrypted_reasoning.unwrap_or(true),
                limits,
            )?),
            None => None,
        },
        response: match input.response.as_ref() {
            Some(response) => Some(translate_response(response, limits)?),
            None => None,
        },
    })
}

// json.loads of a Responses function call's arguments, at the size the JSON boundary allows
pub fn decode_messages_responses_out(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<Json, MessagesResponsesOutputError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    serde_json::from_slice(bytes).map_err(|source| MessagesResponsesOutputError::Json { source })
}
