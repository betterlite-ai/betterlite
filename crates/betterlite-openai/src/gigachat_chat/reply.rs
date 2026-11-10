use std::collections::BTreeMap;

use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use serde::Deserialize;
use serde_json::Value;
use serde_json::error::Category;

use crate::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use crate::chat::{FunctionCall, ToolCall};
use crate::gigachat_chat::config::GigachatChatConfig;
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::{
    CHAT_COMPLETION_ID_CHARS, IdSource, TOOL_CALL_ID_CHARS, TOOL_CALL_ID_PREFIX, TokenUsage,
    arguments_text, convert_usage,
};

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

struct Converted {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
    finish_reason: Option<String>,
}

pub fn transform_response(
    config: &GigachatChatConfig<'_>,
    input: &ReplyInput<'_>,
) -> Result<ChatResponse, GigaChatError> {
    let RawReply {
        id,
        created,
        choices,
        usage,
    } = decode_reply(input)?;
    let converted_choices = choices
        .into_iter()
        .map(|choice| response_choice(choice, input.structured_output, config.ids))
        .collect();
    Ok(ChatResponse {
        id: response_id(id, config.ids),
        object: String::from("chat.completion"),
        created: created.unwrap_or(input.created),
        model: input.model.to_owned(),
        choices: converted_choices,
        usage: chat_usage(&usage.unwrap_or_default())?,
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
    let raw = message.unwrap_or_default();
    let converted = convert_function_call(&raw, finish_reason.as_deref(), structured_output, ids);
    ChatChoice {
        index: index.unwrap_or_default(),
        message: AssistantMessage {
            role: raw.role.unwrap_or_else(|| String::from("assistant")),
            content: converted.content,
            tool_calls: converted.tool_calls,
            annotations: None,
            audio: None,
            refusal: None,
            provider_specific_fields: MessageProviderFields::default(),
        },
        finish_reason: Some(
            converted
                .finish_reason
                .unwrap_or_else(|| String::from("stop")),
        ),
        logprobs: None,
        provider_specific_fields: BTreeMap::new(),
    }
}

fn response_id(id: Option<String>, ids: &dyn IdSource) -> String {
    id.unwrap_or_else(|| {
        let hex: String = ids
            .new_uuid()
            .simple()
            .to_string()
            .chars()
            .take(CHAT_COMPLETION_ID_CHARS)
            .collect();
        format!("{CHAT_COMPLETION_ID_PREFIX}{hex}")
    })
}

// a reply that finishes on a function call becomes tool calls, or the message text when the request was structured
fn convert_function_call(
    message: &RawMessage,
    finish_reason: Option<&str>,
    structured_output: bool,
    ids: &dyn IdSource,
) -> Converted {
    let unchanged = Converted {
        content: message.content.clone(),
        tool_calls: message.tool_calls.clone(),
        finish_reason: finish_reason.map(str::to_owned),
    };
    if finish_reason != Some("function_call") {
        return unchanged;
    }
    let Some(call) = message
        .function_call
        .as_ref()
        .and_then(Value::as_object)
        .filter(|call| !call.is_empty())
    else {
        return unchanged;
    };
    let arguments = arguments_text(call.get("arguments"));
    if structured_output {
        return Converted {
            content: Some(arguments),
            tool_calls: None,
            finish_reason: Some(String::from("stop")),
        };
    }
    let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
    Converted {
        content: message.content.clone(),
        tool_calls: Some(vec![tool_call(ids, name, arguments)]),
        finish_reason: Some(String::from("tool_calls")),
    }
}

fn tool_call(ids: &dyn IdSource, name: &str, arguments: String) -> ToolCall {
    let hex: String = ids
        .new_uuid()
        .simple()
        .to_string()
        .chars()
        .take(TOOL_CALL_ID_CHARS)
        .collect();
    ToolCall {
        id: format!("{TOOL_CALL_ID_PREFIX}{hex}"),
        kind: String::from("function"),
        function: FunctionCall {
            name: name.to_owned(),
            arguments,
        },
    }
}

fn decode_reply(input: &ReplyInput<'_>) -> Result<RawReply, GigaChatError> {
    match serde_json::from_slice::<RawReply>(input.body) {
        Ok(reply) => Ok(reply),
        Err(source) if matches!(source.classify(), Category::Syntax | Category::Eof) => {
            Err(GigaChatError {
                status_code: input.status_code,
                message: format!("the GigaChat reply is not valid JSON: {source}"),
                headers: None,
            })
        }
        Err(source) => Err(GigaChatError {
            status_code: 500,
            message: format!("the GigaChat reply does not hold the declared fields: {source}"),
            headers: None,
        }),
    }
}

fn chat_usage(usage: &TokenUsage) -> Result<ChatUsage, GigaChatError> {
    let converted = convert_usage(usage).map_err(|source| GigaChatError {
        status_code: 500,
        message: format!("the GigaChat reply usage cannot be read: {source}"),
        headers: None,
    })?;
    let prompt_tokens_details = converted
        .cached_tokens
        .map(|cached| BTreeMap::from([(String::from("cached_tokens"), Some(cached))]));
    Ok(ChatUsage {
        prompt_tokens: converted.prompt_tokens,
        completion_tokens: converted.completion_tokens,
        total_tokens: converted.total_tokens,
        prompt_tokens_details,
        completion_tokens_details: None,
    })
}
