use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::chat::ToolCall;
use crate::chat::choice_finish_reason;
use crate::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};

use super::ReplySource;
#[derive(Debug, Deserialize)]
struct ReplyWire {
    id: Option<String>,
    created: Option<u64>,
    model: Option<String>,
    choices: Option<Vec<ChoiceWire>>,
    usage: Option<UsageWire>,
    system_fingerprint: Option<String>,
    service_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChoiceWire {
    index: Option<u32>,
    message: Option<MessageWire>,
    finish_reason: Option<String>,
    logprobs: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
struct MessageWire {
    role: Option<String>,
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
    annotations: Option<Vec<Value>>,
    audio: Option<Value>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
struct UsageWire {
    #[serde(rename = "prompt_tokens")]
    prompt: Option<u64>,
    #[serde(rename = "completion_tokens")]
    completion: Option<u64>,
    #[serde(rename = "total_tokens")]
    total: Option<u64>,
}

// ModelResponse(**body): choices that are not a list become one default choice
pub(super) fn response_from_wire(
    reply: Value,
    source: &ReplySource,
) -> Result<ChatResponse, serde_json::Error> {
    let wire: ReplyWire = serde_json::from_value(reply)?;
    Ok(ChatResponse {
        id: wire
            .id
            .unwrap_or_else(|| format!("chatcmpl-{}", (source.identifier)())),
        object: String::from("chat.completion"),
        created: wire
            .created
            .unwrap_or_else(|| u64::try_from((source.clock)()).unwrap_or_default()),
        model: wire.model.unwrap_or_default(),
        choices: wire.choices.map_or_else(
            || {
                vec![ChatChoice {
                    index: 0,
                    message: message_from_wire(MessageWire::default()),
                    finish_reason: Some(String::from("stop")),
                    logprobs: None,
                    provider_specific_fields: BTreeMap::new(),
                }]
            },
            |choices| choices.into_iter().map(choice_from_wire).collect(),
        ),
        usage: wire.usage.map_or_else(ChatUsage::default, usage_from_wire),
        system_fingerprint: wire.system_fingerprint,
        service_tier: wire.service_tier,
    })
}

fn choice_from_wire(choice: ChoiceWire) -> ChatChoice {
    let (finish_reason, native) = choice_finish_reason(choice.finish_reason.as_deref());
    ChatChoice {
        index: choice.index.unwrap_or_default(),
        message: message_from_wire(choice.message.unwrap_or_default()),
        finish_reason: Some(finish_reason),
        logprobs: choice.logprobs,
        provider_specific_fields: native,
    }
}

fn message_from_wire(message: MessageWire) -> AssistantMessage {
    AssistantMessage {
        role: message
            .role
            .filter(|role| !role.is_empty())
            .unwrap_or_else(|| String::from("assistant")),
        content: message.content,
        tool_calls: message.tool_calls.filter(|calls| !calls.is_empty()),
        annotations: message.annotations,
        audio: message.audio,
        refusal: None,
        provider_specific_fields: MessageProviderFields::default(),
    }
}

// Usage turns a null count into zero, and a null usage is the absent one
fn usage_from_wire(usage: UsageWire) -> ChatUsage {
    ChatUsage {
        prompt_tokens: usage.prompt.unwrap_or_default(),
        completion_tokens: usage.completion.unwrap_or_default(),
        total_tokens: usage.total.unwrap_or_default(),
        prompt_tokens_details: None,
        completion_tokens_details: None,
    }
}
