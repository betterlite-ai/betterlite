use serde::{Deserialize, Serialize};

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_chat_output::output::MessageResponse;

use super::error::MessagesResponsesOutputError;
use super::outputs::ResponsesRequestRecord;
use super::records::{AnthropicRequestRecord, ResponseRecord, SettingsRecord};
use super::request::translate_request;
use super::response::translate_response;

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
    let include = input.include_encrypted_reasoning.unwrap_or(true);
    let request = input
        .request
        .as_ref()
        .map(|request| translate_request(request, &input.settings, include, limits))
        .transpose()?;
    let response = input
        .response
        .as_ref()
        .map(|response| translate_response(response, limits))
        .transpose()?;
    Ok(MessagesResponsesOutpuResult { request, response })
}
