use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::chat::ToolCall;
use crate::chat::choice_finish_reason;
use crate::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use crate::compatible_wire::{NEW_ID, system_clock};

#[derive(Deserialize)]
struct ReplyWire {
    id: Option<String>,
    created: Option<u64>,
    model: Option<String>,
    choices: Option<Vec<ChoiceWire>>,
    usage: Option<UsageWire>,
}

#[derive(Deserialize)]
struct ChoiceWire {
    index: u32,
    message: Option<MessageWire>,
    finish_reason: Option<String>,
    logprobs: Option<Value>,
}

#[derive(Deserialize, Default)]
struct MessageWire {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize)]
struct UsageWire {
    #[serde(rename = "prompt_tokens")]
    prompt: Option<u64>,
    #[serde(rename = "completion_tokens")]
    completion: Option<u64>,
    #[serde(rename = "total_tokens")]
    total: Option<u64>,
}

// ModelResponse(**body): a null or absent id, created and model take their defaults, and Usage turns a null count into zero
pub(super) fn response_from_wire(reply: Value) -> Result<ChatResponse, serde_json::Error> {
    let wire: ReplyWire = serde_json::from_value(reply)?;
    Ok(ChatResponse {
        id: wire.id.unwrap_or_else(|| format!("chatcmpl-{}", NEW_ID())),
        object: String::from("chat.completion"),
        created: wire
            .created
            .unwrap_or_else(|| u64::try_from(system_clock()).unwrap_or_default()),
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
        usage: wire
            .usage
            .map_or_else(ChatUsage::default, |usage| ChatUsage {
                prompt_tokens: usage.prompt.unwrap_or_default(),
                completion_tokens: usage.completion.unwrap_or_default(),
                total_tokens: usage.total.unwrap_or_default(),
                prompt_tokens_details: None,
                completion_tokens_details: None,
            }),
        system_fingerprint: None,
        service_tier: None,
    })
}

fn choice_from_wire(choice: ChoiceWire) -> ChatChoice {
    ChatChoice {
        index: choice.index,
        message: message_from_wire(choice.message.unwrap_or_default()),
        finish_reason: Some(choice_finish_reason(choice.finish_reason.as_deref()).0),
        logprobs: choice.logprobs,
        provider_specific_fields: BTreeMap::new(),
    }
}

fn message_from_wire(message: MessageWire) -> AssistantMessage {
    AssistantMessage {
        role: if message.role.is_empty() {
            String::from("assistant")
        } else {
            message.role
        },
        content: message.content,
        tool_calls: message.tool_calls.filter(|calls| !calls.is_empty()),
        annotations: None,
        audio: None,
        refusal: None,
        provider_specific_fields: MessageProviderFields::default(),
    }
}
