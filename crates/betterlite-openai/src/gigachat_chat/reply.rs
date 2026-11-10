use std::collections::BTreeMap;

use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use serde::Deserialize;
use serde_json::Value;
use serde_json::error::Category;

use crate::chat::choice_finish_reason;
use crate::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use crate::chat::{FunctionCall, ToolCall};
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::{
    CHAT_COMPLETION_ID_CHARS, IdSource, TOOL_CALL_ID_CHARS, TOOL_CALL_ID_PREFIX, TokenUsage,
    arguments_text, convert_usage, generated_id,
};
use crate::openai_base_config::Object;

pub struct ReplyInput<'body> {
    pub status_code: u16,
    pub body: &'body [u8],
    pub model: &'body str,
    pub created: u64,
    pub structured_output: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawReply {
    id: Option<String>,
    created: Option<u64>,
    choices: Vec<RawChoice>,
    usage: Option<TokenUsage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawChoice {
    index: Option<u32>,
    message: Option<RawMessage>,
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawMessage {
    role: Option<String>,
    content: Option<String>,
    function_call: Option<Value>,
    tool_calls: Option<Vec<ToolCall>>,
}

pub fn transform_response(
    ids: &dyn IdSource,
    input: &ReplyInput<'_>,
) -> Result<ChatResponse, GigaChatError> {
    let reply = serde_json::from_slice::<Object>(input.body)
        .and_then(|fields| serde_json::from_value::<RawReply>(Value::Object(fields)))
        .map_err(|source| GigaChatError {
            status_code: if matches!(source.classify(), Category::Syntax | Category::Eof) {
                input.status_code
            } else {
                500
            },
            message: format!("the GigaChat reply cannot be read: {source}"),
            headers: None,
            source: Some(source),
        })?;
    Ok(ChatResponse {
        id: reply.id.unwrap_or_else(|| {
            generated_id(ids, CHAT_COMPLETION_ID_PREFIX, CHAT_COMPLETION_ID_CHARS)
        }),
        object: String::from("chat.completion"),
        created: reply.created.unwrap_or(input.created),
        model: input.model.to_owned(),
        choices: reply
            .choices
            .into_iter()
            .map(|choice| response_choice(choice, input.structured_output, ids))
            .collect(),
        usage: chat_usage(&reply.usage.unwrap_or_default()),
        system_fingerprint: None,
        service_tier: None,
    })
}

fn response_choice(choice: RawChoice, structured_output: bool, ids: &dyn IdSource) -> ChatChoice {
    let RawChoice {
        index,
        message,
        finish_reason,
    } = choice;
    let mut raw = message.unwrap_or_default();
    let (tool_calls, content, reason) =
        convert_function_call(&mut raw, finish_reason, structured_output, ids);
    let (mapped_reason, provider_specific_fields) = choice_finish_reason(reason.as_deref());
    ChatChoice {
        index: index.unwrap_or_default(),
        message: AssistantMessage {
            role: raw.role.unwrap_or_else(|| String::from("assistant")),
            content,
            tool_calls,
            annotations: None,
            audio: None,
            refusal: None,
            provider_specific_fields: MessageProviderFields::default(),
        },
        finish_reason: Some(mapped_reason),
        logprobs: None,
        provider_specific_fields,
    }
}

fn convert_function_call(
    raw: &mut RawMessage,
    finish_reason: Option<String>,
    structured_output: bool,
    ids: &dyn IdSource,
) -> (Option<Vec<ToolCall>>, Option<String>, Option<String>) {
    let function_call = raw.function_call.take().and_then(|call| {
        call.as_object()
            .filter(|fields| !fields.is_empty())
            .cloned()
    });
    let Some(call) = function_call.filter(|_| finish_reason.as_deref() == Some("function_call"))
    else {
        return (raw.tool_calls.take(), raw.content.take(), finish_reason);
    };
    let arguments = arguments_text(call.get("arguments"));
    if structured_output {
        return (None, Some(arguments), Some(String::from("stop")));
    }
    let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
    let tool_call = ToolCall {
        id: generated_id(ids, TOOL_CALL_ID_PREFIX, TOOL_CALL_ID_CHARS),
        kind: String::from("function"),
        function: FunctionCall {
            name: name.to_owned(),
            arguments,
        },
    };
    (
        Some(vec![tool_call]),
        raw.content.take(),
        Some(String::from("tool_calls")),
    )
}

fn chat_usage(usage: &TokenUsage) -> ChatUsage {
    let converted = convert_usage(usage);
    ChatUsage {
        prompt_tokens: converted.prompt_tokens,
        completion_tokens: converted.completion_tokens,
        total_tokens: converted.total_tokens,
        prompt_tokens_details: converted
            .cached_tokens
            .map(|cached| BTreeMap::from([(String::from("cached_tokens"), Some(cached))])),
        completion_tokens_details: None,
    }
}
