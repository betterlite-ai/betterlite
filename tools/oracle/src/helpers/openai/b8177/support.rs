use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_openai::chat::ToolCall;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::gigachat_chat::config::{GigachatBearer, GigachatImageUploader};
use betterlite_openai::gigachat_chat::error::GigaChatError;
use betterlite_openai::gigachat_utils::IdSource;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::HelperError;

pub(crate) const FIXED_UUID: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;

pub(crate) struct FixedIds;

impl IdSource for FixedIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128(FIXED_UUID)
    }
}

pub(crate) struct TokenBearer {
    pub(crate) token: SecretString,
}

#[async_trait]
impl GigachatBearer for TokenBearer {
    async fn access_token(
        &self,
        _credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigaChatError> {
        Ok(self.token.clone())
    }
}

pub(crate) struct Uploader {
    pub(crate) ids: BTreeMap<String, String>,
}

#[async_trait]
impl GigachatImageUploader for Uploader {
    async fn upload(&self, image_url: &str) -> Option<String> {
        self.ids.get(image_url).cloned()
    }
}

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}

pub(crate) fn chat_reply_value(response: &ChatResponse) -> Value {
    let choices: Vec<Value> = response
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "role": choice.message.role,
                "content": choice.message.content.as_deref().map(parsed_text),
                "tool_calls": choice
                    .message
                    .tool_calls
                    .as_ref()
                    .map(|calls| calls.iter().map(tool_call_value).collect::<Vec<Value>>()),
                "finish_reason": choice.finish_reason,
                "provider_specific_fields": Some(&choice.provider_specific_fields)
                    .filter(|fields| !fields.is_empty()),
            })
        })
        .collect();
    json!({
        "ok": true,
        "id": response.id,
        "created": response.created,
        "model": response.model,
        "choices": choices,
        "usage": usage_value(&response.usage),
    })
}

fn tool_call_value(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "type": call.kind,
        "function": {
            "name": call.function.name,
            "arguments": parsed_text(&call.function.arguments),
        },
    })
}

// LiteLLM writes JSON text with json.dumps spacing, so both helpers compare the parsed value
fn parsed_text(text: &str) -> Value {
    match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => Value::String(text.to_owned()),
    }
}

pub(crate) fn settled_body(mut body: Value) -> Value {
    if let Some(Value::Array(messages)) = body.get_mut("messages") {
        for message in messages {
            if message.get("role").and_then(Value::as_str) != Some("function") {
                continue;
            }
            if let Some(content) = message.get_mut("content")
                && let Some(text) = content.as_str()
            {
                *content = parsed_text(text);
            }
        }
    }
    body
}

fn settled_tool_use(mut tool_use: Value) -> Value {
    if let Some(arguments) = tool_use.pointer_mut("/function/arguments")
        && let Some(text) = arguments.as_str()
    {
        *arguments = parsed_text(text);
    }
    tool_use
}

fn usage_value<Usage>(usage: &Usage) -> Value
where
    Usage: Serialize,
{
    let fields = json!(usage);
    json!({
        "prompt_tokens": fields.pointer("/prompt_tokens"),
        "completion_tokens": fields.pointer("/completion_tokens"),
        "total_tokens": fields.pointer("/total_tokens"),
        "cached_tokens": fields.pointer("/prompt_tokens_details/cached_tokens"),
    })
}

pub(crate) fn stream_chunk_value(chunk: &GenericChunk) -> Value {
    json!({
        "text": chunk.text,
        "tool_use": settled_tool_use(json!(chunk.tool_use)),
        "is_finished": chunk.is_finished,
        "finish_reason": chunk.finish_reason,
        "usage": chunk.usage.as_ref().map(usage_value),
        "index": chunk.index,
    })
}
