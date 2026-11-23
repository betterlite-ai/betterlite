use std::collections::BTreeMap;

use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::Extra;
use betterlite_openai::chat::ToolCall;
use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use serde::{Deserialize, Serialize};

use super::error::DeepseekReplyError;

// The ModelResponse LiteLLM passes into and receives back from these transforms; the provider_specific_fields members carry reasoning_content, which the shared chat wire cannot represent.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    #[serde(default)]
    pub id: String,
    #[serde(default = "default_object")]
    pub object: String,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub message: Message,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub provider_specific_fields: Option<Extra>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    pub provider_specific_fields: Option<Extra>,
}

// Python keeps token counts as unbounded ints; these hold the i64 range and no subtotal is recomputed.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
}

fn default_object() -> String {
    String::from("chat.completion")
}

// The Choices constructor applies map_finish_reason, so the seed a transform receives already went through it.
pub(super) fn chat_response(seed: &Reply) -> ChatResponse {
    ChatResponse {
        id: seed.id.clone(),
        object: seed.object.clone(),
        created: seed.created,
        model: seed.model.clone(),
        choices: seed.choices.iter().map(seed_choice).collect(),
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}

fn seed_choice(choice: &Choice) -> ChatChoice {
    let mapped = map_finish_reason(choice.finish_reason.as_deref().unwrap_or("stop"));
    ChatChoice {
        index: choice.index,
        message: AssistantMessage {
            role: choice.message.role.clone(),
            content: choice.message.content.clone(),
            tool_calls: choice
                .message
                .tool_calls
                .as_ref()
                .filter(|calls| !calls.is_empty())
                .cloned(),
            annotations: None,
            audio: None,
            refusal: None,
            provider_specific_fields: MessageProviderFields::default(),
        },
        finish_reason: Some(mapped.reason.as_str().to_owned()),
        logprobs: None,
        provider_specific_fields: BTreeMap::new(),
    }
}

// The base changes content, finish_reason, model, created and usage; every other field comes back as seeded.
pub(super) fn from_chat(response: ChatResponse, seed: &Reply) -> Result<Reply, DeepseekReplyError> {
    let usage = usage_from_chat(&response.usage)?;
    let choices = response
        .choices
        .into_iter()
        .enumerate()
        .map(|(position, choice)| {
            let seeded = seed.choices.get(position);
            Choice {
                index: choice.index,
                message: Message {
                    role: choice.message.role,
                    content: choice.message.content,
                    tool_calls: choice.message.tool_calls,
                    provider_specific_fields: seeded
                        .and_then(|source| source.message.provider_specific_fields.clone()),
                },
                finish_reason: choice.finish_reason,
                provider_specific_fields: seeded
                    .and_then(|source| source.provider_specific_fields.clone()),
            }
        })
        .collect();
    Ok(Reply {
        id: response.id,
        object: response.object,
        created: response.created,
        model: response.model,
        choices,
        usage: Some(usage),
    })
}

fn usage_from_chat(usage: &ChatUsage) -> Result<Usage, DeepseekReplyError> {
    Ok(Usage {
        prompt_tokens: i64::try_from(usage.prompt_tokens).map_err(|_error| {
            DeepseekReplyError::UsageRange {
                name: "prompt_tokens",
            }
        })?,
        completion_tokens: i64::try_from(usage.completion_tokens).map_err(|_error| {
            DeepseekReplyError::UsageRange {
                name: "completion_tokens",
            }
        })?,
        total_tokens: i64::try_from(usage.total_tokens).map_err(|_error| {
            DeepseekReplyError::UsageRange {
                name: "total_tokens",
            }
        })?,
    })
}
