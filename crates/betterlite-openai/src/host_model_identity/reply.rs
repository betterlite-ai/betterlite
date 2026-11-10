use std::collections::BTreeMap;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde_json::Value;
use uuid::Uuid;

use crate::chat::choice_finish_reason;
use crate::chat::response::ChatResponse;
use crate::openai_base_config::Object;

use super::error::HostModelIdentityError;

// the pooled transport hands a decoder at most this many body bytes
pub const MAX_REPLY_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

pub struct PostCall<'request> {
    pub input: &'request Json,
    pub api_key: Option<&'request SecretString>,
    pub original_response: &'request str,
    pub complete_input_dict: &'request Object,
}

pub trait PostCallLogger {
    fn post_call(&mut self, call: PostCall<'_>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostReplyMode<'request> {
    Clarifai {
        request_model: &'request str,
    },
    Compactifai {
        request_model: &'request str,
        json_mode: bool,
    },
}

impl HostReplyMode<'_> {
    const fn host(self) -> &'static str {
        match self {
            Self::Clarifai { .. } => "clarifai",
            Self::Compactifai { .. } => "compactifai",
        }
    }
}

// ModelResponse invents an id and a creation time for a reply that carries neither
#[derive(Debug, Clone, Copy)]
pub struct ReplySource {
    pub identifier: fn() -> Uuid,
    pub clock: fn() -> i64,
}

pub fn decode_host_named_reply(
    bytes: &[u8],
    mode: HostReplyMode<'_>,
    reply_source: &ReplySource,
) -> Result<ChatResponse, HostModelIdentityError> {
    let host = mode.host();
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(HostModelIdentityError::TooLarge {
            host,
            size: bytes.len(),
            limit: MAX_REPLY_BYTES,
        });
    }
    let mut reply: Value = serde_json::from_slice(bytes)
        .map_err(|source| HostModelIdentityError::Json { host, source })?;
    if let HostReplyMode::Compactifai {
        json_mode: true, ..
    } = mode
    {
        convert_single_tool_calls(&mut reply).ok_or(HostModelIdentityError::Malformed { host })?;
    }
    let model_is_absent = reply.get("model").is_none_or(Value::is_null);
    let mut native_reasons = Vec::new();
    let normalized = model_response_value(reply, &mut native_reasons, reply_source)
        .ok_or(HostModelIdentityError::Malformed { host })?;
    let mut response: ChatResponse = serde_json::from_value(normalized)
        .map_err(|source| HostModelIdentityError::Reply { host, source })?;
    for (choice, native_fields) in response.choices.iter_mut().zip(native_reasons) {
        choice.provider_specific_fields = native_fields;
    }
    apply_model_identity(&mut response, model_is_absent, mode);
    Ok(response)
}

// CompactifAI's json mode moves one tool call's arguments into content, and an explicit null stays null
fn convert_single_tool_calls(reply: &mut Value) -> Option<()> {
    let choices = reply.get_mut("choices").and_then(Value::as_array_mut)?;
    for choice in choices {
        let choice_fields = choice.as_object_mut()?;
        let Some(message) = choice_fields.get_mut("message") else {
            continue;
        };
        if message.is_null() {
            continue;
        }
        let message_fields = message.as_object_mut()?;
        if message_fields.is_empty() {
            continue;
        }
        let Some(tool_calls) = message_fields.get_mut("tool_calls") else {
            continue;
        };
        if tool_calls.is_null() {
            continue;
        }
        let content = {
            let calls = tool_calls.as_array()?;
            if calls.len() != 1 {
                continue;
            }
            let call = calls.first().and_then(Value::as_object)?;
            let function = call.get("function").and_then(Value::as_object)?;
            function
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()))
        };
        message_fields.insert(String::from("content"), content);
        message_fields.insert(String::from("tool_calls"), Value::Null);
    }
    Some(())
}

// ModelResponse(**body): only a list of choices is kept, anything else becomes one default choice
fn model_response_value(
    reply: Value,
    native_reasons: &mut Vec<BTreeMap<String, Value>>,
    reply_source: &ReplySource,
) -> Option<Value> {
    let Value::Object(mut fields) = reply else {
        return None;
    };
    if fields.get("id").is_none_or(Value::is_null) {
        fields.insert(
            String::from("id"),
            Value::String(format!("chatcmpl-{}", (reply_source.identifier)())),
        );
    }
    if fields.get("created").is_none_or(Value::is_null) {
        fields.insert(String::from("created"), Value::from((reply_source.clock)()));
    }
    if fields.get("model").is_none_or(Value::is_null) {
        fields.insert(String::from("model"), Value::String(String::new()));
    }
    fields.insert(
        String::from("object"),
        Value::String(String::from("chat.completion")),
    );
    normalize_usage(&mut fields);
    normalize_choices(&mut fields, native_reasons);
    Some(Value::Object(fields))
}

// Usage turns a null count into zero, and a null usage is the absent one
fn normalize_usage(fields: &mut Object) {
    if fields.get("usage").is_some_and(Value::is_null) {
        fields.remove("usage");
    }
    let Some(usage) = fields.get_mut("usage").and_then(Value::as_object_mut) else {
        return;
    };
    for name in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        if usage.get(name).is_none_or(Value::is_null) {
            usage.insert(String::from(name), Value::from(0_u64));
        }
    }
}

fn normalize_choices(fields: &mut Object, native_reasons: &mut Vec<BTreeMap<String, Value>>) {
    let mut choices = match fields.remove("choices") {
        Some(Value::Array(given)) => given,
        Some(_) | None => vec![Value::Object(Object::from_iter([
            (String::from("index"), Value::from(0_u64)),
            (
                String::from("finish_reason"),
                Value::String(String::from("stop")),
            ),
            (
                String::from("message"),
                Value::Object(Object::from_iter([
                    (
                        String::from("role"),
                        Value::String(String::from("assistant")),
                    ),
                    (String::from("content"), Value::Null),
                ])),
            ),
        ]))],
    };
    for choice in &mut choices {
        native_reasons.push(normalize_choice(choice));
    }
    fields.insert(String::from("choices"), Value::Array(choices));
}

fn normalize_choice(choice: &mut Value) -> BTreeMap<String, Value> {
    let Value::Object(fields) = choice else {
        return BTreeMap::new();
    };
    if fields.get("index").is_none_or(Value::is_null) {
        fields.insert(String::from("index"), Value::from(0_u64));
    }
    normalize_message(fields);
    match fields.remove("finish_reason") {
        None | Some(Value::Null) => {
            fields.insert(
                String::from("finish_reason"),
                Value::String(String::from("stop")),
            );
            BTreeMap::new()
        }
        Some(Value::String(reason)) => {
            let (mapped, native) = choice_finish_reason(Some(&reason));
            fields.insert(String::from("finish_reason"), Value::String(mapped));
            native
        }
        Some(other) => {
            fields.insert(String::from("finish_reason"), other);
            BTreeMap::new()
        }
    }
}

fn normalize_message(choice: &mut Object) {
    let mut message = match choice.remove("message") {
        Some(Value::Object(given)) => given,
        Some(Value::Null) | None => Object::from_iter([
            (
                String::from("role"),
                Value::String(String::from("assistant")),
            ),
            (String::from("content"), Value::Null),
        ]),
        Some(other) => {
            choice.insert(String::from("message"), other);
            return;
        }
    };
    if message
        .get("role")
        .is_none_or(|role| role.is_null() || role.as_str() == Some(""))
    {
        message.insert(
            String::from("role"),
            Value::String(String::from("assistant")),
        );
    }
    if message
        .get("tool_calls")
        .is_some_and(|calls| calls.as_array().is_some_and(Vec::is_empty))
    {
        message.insert(String::from("tool_calls"), Value::Null);
    }
    choice.insert(String::from("message"), Value::Object(message));
}

fn apply_model_identity(
    response: &mut ChatResponse,
    model_is_absent: bool,
    mode: HostReplyMode<'_>,
) {
    match mode {
        HostReplyMode::Clarifai { request_model } => {
            if !model_is_absent {
                response.model = format!("clarifai/{request_model}");
            }
        }
        HostReplyMode::Compactifai { request_model, .. } => {
            response.model = format!("compactifai/{request_model}");
        }
    }
}
