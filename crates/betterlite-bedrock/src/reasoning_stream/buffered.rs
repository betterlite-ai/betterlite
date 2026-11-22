use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::compatible_wire::error::CompatibleWireError;
use betterlite_openai::compatible_wire::{
    CompatibleReply, LogInput, ReplyOptions, transform_response as parent_transform_response,
};
use indexmap::IndexMap;
use serde::Deserialize;
use thiserror::Error;

use super::split_reasoning_tag;

#[derive(Debug, Error)]
pub enum BufferedError {
    #[error(transparent)]
    Reply(#[from] CompatibleWireError),
    #[error("the chat reply's reasoning fields do not have the shape of a chat completion reply")]
    Reasoning(#[source] serde_json::Error),
}

pub struct ReplyInput<'reply> {
    pub options: ReplyOptions<'reply>,
    pub provider_headers: &'reply IndexMap<String, String>,
    pub existing_additional_headers: &'reply AdditionalHeaders,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeChoice {
    pub index: u32,
    pub finish_reason: Option<String>,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeReply {
    pub response: ChatResponse,
    pub hidden_model: Option<String>,
    pub headers: ProviderHeaderProjection,
    pub choices: Vec<RuntimeChoice>,
}

#[derive(Deserialize)]
struct RawReply {
    #[serde(default)]
    choices: Vec<RawChoice>,
}

#[derive(Default, Deserialize)]
struct RawChoice {
    #[serde(default)]
    message: RawMessage,
}

#[derive(Default, Deserialize)]
struct RawMessage {
    #[serde(default)]
    reasoning_content: Option<String>,
}

pub fn transform_response(
    bytes: &[u8],
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    input: LogInput,
    reply: &ReplyInput<'_>,
) -> Result<RuntimeReply, BufferedError> {
    let parent: CompatibleReply =
        parent_transform_response(bytes, &reply.options, state, env, input)?;
    let headers =
        set_provider_response_headers(reply.existing_additional_headers, reply.provider_headers);
    let raw: RawReply = serde_json::from_slice(bytes).map_err(BufferedError::Reasoning)?;
    let mut response = parent.response;
    let choices = split_choices(&mut response, &raw);
    Ok(RuntimeReply {
        response,
        hidden_model: parent.hidden_model,
        headers,
        choices,
    })
}

fn split_choices(response: &mut ChatResponse, raw: &RawReply) -> Vec<RuntimeChoice> {
    let mut choices = Vec::with_capacity(response.choices.len());
    for (position, choice) in response.choices.iter_mut().enumerate() {
        let mut reasoning_content = raw
            .choices
            .get(position)
            .and_then(|entry| entry.message.reasoning_content.clone());
        let Some(content) = choice.message.content.as_deref() else {
            choices.push(RuntimeChoice {
                index: choice.index,
                finish_reason: choice.finish_reason.clone(),
                content: None,
                reasoning_content,
            });
            continue;
        };
        let (reasoning, body) = split_reasoning_tag(content);
        if let Some(extracted) = reasoning {
            let mut combined = reasoning_content.take().unwrap_or_default();
            combined.push_str(&extracted);
            reasoning_content = Some(combined);
        }
        choice.message.content = Some(body.clone());
        choices.push(RuntimeChoice {
            index: choice.index,
            finish_reason: choice.finish_reason.clone(),
            content: Some(body),
            reasoning_content,
        });
    }
    choices
}
