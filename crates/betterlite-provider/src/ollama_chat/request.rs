use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Message, convert_content_list_to_str};
use betterlite_prompts::message_fields::image_hoist::extract_images_from_message;
use betterlite_prompts::reasoning_fields::content_split::extract_reasoning_content;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use serde_with::skip_serializing_none;

use super::config::OllamaChatConfig;
use super::error::OllamaError;
use super::function::FUNCTION_TYPE;

const REASONING_FIELDS: [&str; 2] = ["reasoning_content", "reasoning"];

pub struct RequestInput<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request IndexMap<String, Json>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    pub options: IndexMap<String, Json>,
    pub stream: bool,
    pub format: Option<Json>,
    pub tools: Option<Json>,
    pub keep_alive: Option<Json>,
    pub think: Option<Json>,
    // LiteLLM keeps the popped function name on litellm_params for transform_response to read
    #[serde(skip)]
    pub function_name: Option<Json>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaMessage {
    pub role: String,
    pub thinking: Option<String>,
    pub content: String,
    pub images: Vec<String>,
    pub tool_calls: Option<Vec<OllamaToolCall>>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaToolCall {
    pub function: OllamaCallFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaCallFunction {
    pub name: String,
    pub arguments: Json,
}

#[derive(Debug, Deserialize)]
struct RequestToolCall {
    #[serde(rename = "type")]
    kind: String,
    function: RequestFunction,
}

#[derive(Debug, Deserialize)]
struct RequestFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

pub fn transform_request(
    input: &RequestInput<'_>,
    config: &OllamaChatConfig,
) -> Result<OllamaChatRequest, OllamaError> {
    let mut optional = input.optional_params.clone();
    let stream = match optional.shift_remove("stream") {
        Some(Json::Bool(value)) => value,
        Some(Json::Null) | None => false,
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(OllamaError::Param { field: "stream" });
        }
    };
    let format = take_nullable(&mut optional, "format");
    let keep_alive = take_nullable(&mut optional, "keep_alive");
    let think = take_nullable(&mut optional, "think");
    let function_name = take_nullable(&mut optional, "function_name");
    let tools = take_nullable(&mut optional, "tools");
    let mut messages = Vec::with_capacity(input.messages.len());
    for (index, message) in input.messages.iter().enumerate() {
        messages.push(ollama_message(message, index)?);
    }
    for (name, value) in config.get_config() {
        optional.entry(name).or_insert(value);
    }
    Ok(OllamaChatRequest {
        model: input.model.to_owned(),
        messages,
        options: optional,
        stream,
        format,
        tools,
        keep_alive,
        think,
        function_name,
    })
}

fn take_nullable(optional: &mut IndexMap<String, Json>, name: &str) -> Option<Json> {
    match optional.shift_remove(name) {
        Some(Json::Null) | None => None,
        Some(value) => Some(value),
    }
}

fn ollama_message(message: &Message, index: usize) -> Result<OllamaMessage, OllamaError> {
    let fields = &message.provider_specific_fields;
    let explicit = first_text(fields, &REASONING_FIELDS, index)?;
    // a present reasoning key, even null, stops LiteLLM from reading the content for think tags
    let thinking = if REASONING_FIELDS
        .iter()
        .any(|name| fields.contains_key(*name))
    {
        explicit
    } else {
        extract_reasoning_content(message).0
    };
    Ok(OllamaMessage {
        role: message.role.clone(),
        thinking,
        content: convert_content_list_to_str(message),
        images: extract_images_from_message(message),
        tool_calls: tool_calls_from_message(message, index)?,
        tool_call_id: first_text(fields, &["tool_call_id"], index)?,
    })
}

fn first_text(
    fields: &Object,
    names: &[&'static str],
    index: usize,
) -> Result<Option<String>, OllamaError> {
    for name in names {
        match fields.get(*name) {
            Some(Json::Text(value)) => return Ok(Some(value.clone())),
            Some(Json::Null) => return Ok(None),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(OllamaError::RequestField { index, field: name });
            }
            None => {}
        }
    }
    Ok(None)
}

fn tool_calls_from_message(
    message: &Message,
    index: usize,
) -> Result<Option<Vec<OllamaToolCall>>, OllamaError> {
    let Some(value) = message.provider_specific_fields.get("tool_calls") else {
        return Ok(None);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let raw = serde_json::to_value(value).map_err(|source| OllamaError::Shape { source })?;
    let calls: Vec<RequestToolCall> =
        serde_json::from_value(raw).map_err(|source| OllamaError::RequestFieldShape {
            index,
            field: "tool_calls",
            source,
        })?;
    let mut converted = Vec::with_capacity(calls.len());
    for call in calls {
        if call.kind != FUNCTION_TYPE {
            continue;
        }
        let arguments = match call.function.arguments {
            Some(text) => serde_json::from_str::<Json>(&text)
                .map_err(|source| OllamaError::RequestToolArguments { index, source })?,
            None => Json::Object(IndexMap::new()),
        };
        converted.push(OllamaToolCall {
            function: OllamaCallFunction {
                name: call.function.name.unwrap_or_default(),
                arguments,
            },
        });
    }
    Ok(Some(converted))
}
