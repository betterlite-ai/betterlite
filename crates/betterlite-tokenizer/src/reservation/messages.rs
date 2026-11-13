use std::cmp::Ordering;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::count::Counter;
use crate::count::error::CountError;
use crate::count::request::{
    ContentPart, FunctionCall, Message, MessageContent, NamedToolChoice, Part, TokenRequest,
    ToolCall, ToolCallFunction, ToolChoice, ToolFunctionName,
};

use super::RequestBody;
use super::ReservationError;
use super::python_text;
use super::text;

// token_counter reports a content-list or tool-call shape problem as ValueError, which counts the original value as text
enum MessageFailure {
    ValueError,
    Unavailable,
}

pub(crate) async fn count_messages_field(
    counter: &Counter,
    body: &RequestBody,
    model: &str,
    messages_value: &Json,
) -> Result<Option<usize>, ReservationError> {
    let messages = match parse_messages(messages_value) {
        Ok(messages) => messages,
        Err(MessageFailure::ValueError) => {
            return Ok(Some(
                text::count_text_tokens(counter, model, messages_value).await?,
            ));
        }
        Err(MessageFailure::Unavailable) => return Err(ReservationError::Messages),
    };
    let tools = parse_tools(body.get("tools"))?;
    let tool_choice = parse_tool_choice(body.get("tool_choice"))?;
    let request = TokenRequest {
        model,
        custom_tokenizer: None,
        text: None,
        messages: Some(&messages),
        count_response_tokens: Some(false),
        tools: tools.as_deref(),
        tool_choice: tool_choice.as_ref(),
    };
    match counter.count(&request).await {
        Ok(count) => Ok(Some(count)),
        Err(CountError::UnsupportedContent { .. } | CountError::ContentField { .. }) => Ok(Some(
            text::count_text_tokens(counter, model, messages_value).await?,
        )),
        Err(error) => Err(ReservationError::from(error)),
    }
}

fn parse_messages(value: &Json) -> Result<Vec<Message>, MessageFailure> {
    if is_falsey(value) {
        return Ok(Vec::new());
    }
    let Json::List(items) = value else {
        return Err(MessageFailure::Unavailable);
    };
    items.iter().map(parse_message).collect()
}

fn parse_message(value: &Json) -> Result<Message, MessageFailure> {
    let Json::Object(fields) = value else {
        return Err(MessageFailure::Unavailable);
    };
    let Some(Json::Text(role)) = fields.get("role") else {
        return Err(MessageFailure::Unavailable);
    };
    let mut passthrough = IndexMap::new();
    for (key, field) in fields {
        if !is_known_key(key)
            && let Json::Text(text) = field
        {
            passthrough.insert(key.clone(), Json::Text(text.clone()));
        }
    }
    Ok(Message {
        role: role.clone(),
        content: parse_content(fields.get("content"))?,
        name: text_field(fields.get("name")),
        tool_calls: parse_tool_calls(fields.get("tool_calls"))?,
        function_call: parse_function_call(fields.get("function_call"))?,
        tool_call_id: text_field(fields.get("tool_call_id")),
        reasoning_content: text_field(fields.get("reasoning_content")),
        search_results: fields.get("search_results").cloned(),
        passthrough,
    })
}

// the fields B196 reads through their own branch; every other string value counts as plain text
fn is_known_key(key: &str) -> bool {
    matches!(
        key,
        "role"
            | "content"
            | "name"
            | "tool_calls"
            | "function_call"
            | "tool_call_id"
            | "reasoning_content"
            | "search_results"
    )
}

fn text_field(value: Option<&Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn parse_content(value: Option<&Json>) -> Result<Option<MessageContent>, MessageFailure> {
    match value {
        Some(Json::Text(text)) => Ok(Some(MessageContent::Text(text.clone()))),
        Some(Json::List(items)) => Ok(Some(MessageContent::Parts(parse_parts(items)?))),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => Ok(None),
    }
}

fn parse_parts(items: &[Json]) -> Result<Vec<Part>, MessageFailure> {
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Json::Text(text) => parts.push(Part::Text(text.clone())),
            Json::Object(fields) => {
                let Some(Json::Text(kind)) = fields.get("type") else {
                    return Err(MessageFailure::ValueError);
                };
                if let Some(text) = thinking_or_reference_text(kind, fields) {
                    parts.push(Part::Text(text));
                    continue;
                }
                let rest = fields
                    .iter()
                    .filter(|(key, _)| key.as_str() != "type")
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                parts.push(Part::Block(ContentPart {
                    kind: kind.clone(),
                    fields: rest,
                }));
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                return Err(MessageFailure::ValueError);
            }
        }
    }
    Ok(parts)
}

// token_counter counts a thinking block's text and a tool_reference's tool_name, and skips the opaque signature blobs
fn thinking_or_reference_text(kind: &str, fields: &IndexMap<String, Json>) -> Option<String> {
    match kind {
        "thinking" | "redacted_thinking" => Some(
            fields
                .get("thinking")
                .map_or_else(String::new, python_text::str_of),
        ),
        "tool_reference" => Some(
            fields
                .get("tool_name")
                .filter(|name| !is_falsey(name))
                .map_or_else(String::new, python_text::str_of),
        ),
        _ => None,
    }
}

fn parse_tool_calls(value: Option<&Json>) -> Result<Option<Vec<ToolCall>>, MessageFailure> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(calls)) => parse_calls(calls),
        Some(_) => Err(MessageFailure::ValueError),
    }
}

fn parse_calls(calls: &[Json]) -> Result<Option<Vec<ToolCall>>, MessageFailure> {
    let mut parsed = Vec::with_capacity(calls.len());
    for call in calls {
        let Json::Object(fields) = call else {
            return Err(MessageFailure::Unavailable);
        };
        let Some(function_value) = fields.get("function") else {
            return Err(MessageFailure::ValueError);
        };
        let Json::Object(function) = function_value else {
            return Err(MessageFailure::Unavailable);
        };
        parsed.push(ToolCall {
            function: ToolCallFunction {
                arguments: arguments(function)?,
            },
        });
    }
    Ok(Some(parsed))
}

fn parse_function_call(value: Option<&Json>) -> Result<Option<FunctionCall>, MessageFailure> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(FunctionCall {
            arguments: arguments(fields)?,
        })),
        Some(_) => Err(MessageFailure::ValueError),
    }
}

fn arguments(fields: &IndexMap<String, Json>) -> Result<String, MessageFailure> {
    match fields.get("arguments") {
        None => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(MessageFailure::Unavailable),
    }
}

fn parse_tools(value: Option<&Json>) -> Result<Option<Vec<Json>>, ReservationError> {
    let tools = value.filter(|tools| !is_falsey(tools));
    match tools {
        None => Ok(None),
        Some(Json::List(items)) => Ok(Some(items.clone())),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(ReservationError::Tools)
        }
    }
}

fn parse_tool_choice(value: Option<&Json>) -> Result<Option<ToolChoice>, ReservationError> {
    match value {
        Some(Json::Text(mode)) => Ok(Some(ToolChoice::Mode(mode.clone()))),
        Some(Json::Object(fields)) => named_choice(fields),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => Ok(None),
    }
}

fn named_choice(fields: &IndexMap<String, Json>) -> Result<Option<ToolChoice>, ReservationError> {
    let Some(Json::Object(function)) = fields.get("function") else {
        return Err(ReservationError::ToolChoice);
    };
    let Some(Json::Text(name)) = function.get("name") else {
        return Err(ReservationError::ToolChoice);
    };
    Ok(Some(ToolChoice::Named(NamedToolChoice {
        function: ToolFunctionName { name: name.clone() },
    })))
}

// Python's falsey JSON values: null, false, zero, "" and the empty list or object
fn is_falsey(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(flag) => !flag,
        Json::Number(number) => is_zero(number),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(fields) => fields.is_empty(),
    }
}

fn is_zero(number: &Number) -> bool {
    if number.as_i64().is_some_and(|value| value == 0) {
        return true;
    }
    number
        .as_f64()
        .is_some_and(|value| value.partial_cmp(&0.0_f64) == Some(Ordering::Equal))
}
