use std::pin::Pin;
use std::task::{Context, Poll};

use betterlite_anthropic::response_content::decode_json_mode_arguments;
use betterlite_logging::python_json::dumps;
use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{
    GenericChunk, JsonObject, JsonValue, Usage,
};
use futures_util::Stream;
use serde::{Deserialize, Serialize};

use crate::invoke_events::error::InvokeStreamError;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FakeResponse {
    #[serde(default)]
    pub choices: Vec<FakeChoice>,
    #[serde(default)]
    pub usage: Option<FakeUsage>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FakeChoice {
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub message: FakeMessage,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FakeMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<FakeToolCall>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FakeToolCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: FakeFunction,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FakeFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FakeUsage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
}

// The buffered fake stream LiteLLM builds for a non-streamed ai21-style reply; it yields one chunk or one failure.
pub struct FakeStream {
    pub response: FakeResponse,
    pub json_mode: Option<bool>,
    pub is_done: bool,
}

impl Iterator for FakeStream {
    type Item = Result<GenericChunk, InvokeStreamError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_done {
            return None;
        }
        self.is_done = true;
        Some(parse(&self.response, self.json_mode))
    }
}

impl Stream for FakeStream {
    type Item = Result<GenericChunk, InvokeStreamError>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(Iterator::next(self.get_mut()))
    }
}

fn parse(
    response: &FakeResponse,
    json_mode: Option<bool>,
) -> Result<GenericChunk, InvokeStreamError> {
    let usage = response
        .usage
        .as_ref()
        .ok_or_else(|| decode_error(response, "the buffered response has no usage"))?;
    let choice = response
        .choices
        .first()
        .ok_or_else(|| decode_error(response, "the buffered response has no choices"))?;
    let mut text = choice.message.content.clone().unwrap_or_default();
    let tool_use = match (json_mode, choice.message.tool_calls.as_deref()) {
        (Some(true), Some(calls)) => {
            let first = calls
                .first()
                .ok_or_else(|| decode_error(response, "the first tool call is missing"))?;
            match convert_tool_response_to_message(first.function.arguments.as_deref()) {
                Some(content) => {
                    text = content;
                    None
                }
                None => None,
            }
        }
        (_, Some(calls)) => calls.first().map(tool_use_json),
        (_, None) => None,
    };
    Ok(GenericChunk {
        text,
        tool_use,
        is_finished: true,
        finish_reason: map_finish_reason(choice.finish_reason.as_deref().unwrap_or_default())
            .reason
            .as_str()
            .to_owned(),
        usage: Some(Usage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            cost: None,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            extra: JsonObject::new(),
        }),
        index: Some(0),
        provider_specific_fields: None,
    })
}

// AnthropicConfig._convert_tool_response_to_message: a json.dumps of the values field or of the whole argument object.
fn convert_tool_response_to_message(arguments: Option<&str>) -> Option<String> {
    let raw = arguments?;
    match decode_json_mode_arguments(raw) {
        Err(_source) => Some(raw.to_owned()),
        Ok(Json::Object(fields)) => {
            let values = fields
                .get("values")
                .filter(|value| !matches!(value, Json::Null));
            match values {
                Some(value) => Some(dumps(value)),
                None => Some(dumps(&Json::Object(fields))),
            }
        }
        Ok(other) => Some(dumps(&other)),
    }
}

fn tool_use_json(call: &FakeToolCall) -> JsonValue {
    let mut function = JsonObject::new();
    function.insert(
        "name".to_owned(),
        call.function
            .name
            .as_deref()
            .map_or(JsonValue::Null, |name| JsonValue::String(name.to_owned())),
    );
    function.insert(
        "arguments".to_owned(),
        call.function
            .arguments
            .as_deref()
            .map_or(JsonValue::Null, |arguments| {
                JsonValue::String(arguments.to_owned())
            }),
    );
    let mut tool = JsonObject::new();
    tool.insert(
        "id".to_owned(),
        call.id
            .as_deref()
            .map_or(JsonValue::Null, |id| JsonValue::String(id.to_owned())),
    );
    tool.insert("type".to_owned(), JsonValue::String("function".to_owned()));
    tool.insert("function".to_owned(), JsonValue::Object(function));
    tool.insert(
        "index".to_owned(),
        JsonValue::Number(serde_json::Number::from(0_u8)),
    );
    JsonValue::Object(tool)
}

fn decode_error(response: &FakeResponse, error: &str) -> InvokeStreamError {
    let chunk = serde_json::to_string(response).unwrap_or_default();
    InvokeStreamError::Buffered {
        chunk,
        error: error.to_owned(),
    }
}
