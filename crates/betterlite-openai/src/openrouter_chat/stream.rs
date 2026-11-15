use serde_json::{Map, Value};
use uuid::Uuid;

use crate::chat_choices::stream::chunk_parser as base_chunk_parser;
use crate::chat_choices::wire::StreamReply;
use crate::openai_base_config::Headers;
use crate::openrouter_chat::error::{OpenRouterError, key_error};

// B010's iterator calls this for one parsed chunk; the key reads stay here because LiteLLM's
// override reads them before it builds the same ModelResponseStream the base parser normalizes
pub fn chunk_parser(chunk: &Value) -> Result<StreamReply, OpenRouterError> {
    let raw_choices = checked_choices(chunk)?;
    let mut reply =
        base_chunk_parser(chunk, Uuid::nil, i64::default).map_err(|_source| unexpected(chunk))?;
    // the override assigns delta["reasoning_content"] = delta.get("reasoning") on every choice,
    // so a chunk without the reasoning key clears whatever reasoning_content it carried
    for (choice, raw) in reply.choices.iter_mut().zip(raw_choices) {
        let carries_reasoning = raw
            .as_object()
            .and_then(|fields| fields.get("delta"))
            .is_some_and(|delta| delta.get("reasoning").is_some());
        if !carries_reasoning {
            choice.delta.reasoning_content = None;
        }
    }
    // ModelResponseStream is built without service_tier, so the chunk's own key never reaches it
    reply.service_tier = None;
    Ok(reply)
}

// the override reads choices, each choice's delta, id, created and model in this order, and a
// missing or wrong-shaped one is the KeyError branch rather than the unexpected-response one
fn checked_choices(chunk: &Value) -> Result<&[Value], OpenRouterError> {
    let Some(fields) = chunk.as_object() else {
        return Err(key_error("choices", chunk));
    };
    if let Some(error) = fields.get("error") {
        return Err(stream_error(error, chunk));
    }
    let raw_choices = fields
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| key_error("choices", chunk))?;
    for raw in raw_choices {
        if raw
            .as_object()
            .and_then(|choice| choice.get("delta"))
            .is_none_or(|delta| !delta.is_object())
        {
            return Err(key_error("delta", chunk));
        }
    }
    if fields.get("id").and_then(Value::as_str).is_none() {
        return Err(key_error("id", chunk));
    }
    if fields.get("created").and_then(Value::as_i64).is_none() {
        return Err(key_error("created", chunk));
    }
    if fields.get("model").and_then(Value::as_str).is_none() {
        return Err(key_error("model", chunk));
    }
    Ok(raw_choices)
}

fn stream_error(error: &Value, chunk: &Value) -> OpenRouterError {
    let Some(fields) = error.as_object() else {
        return key_error("message", chunk);
    };
    let Some(message) = fields.get("message").and_then(Value::as_str) else {
        return key_error("message", chunk);
    };
    let Some(code) = fields.get("code") else {
        return key_error("code", chunk);
    };
    let status_code = code
        .as_u64()
        .and_then(|number| u16::try_from(number).ok())
        .unwrap_or(500);
    let metadata = fields
        .get("metadata")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let user_id = fields.get("user_id").and_then(Value::as_str).unwrap_or("");
    let headers = metadata
        .get("headers")
        .and_then(Value::as_object)
        .map(headers)
        .unwrap_or_default();
    let printed = serde_json::to_string(&metadata).unwrap_or_else(|_| String::from("{}"));
    OpenRouterError {
        message: format!("Message: {message}, Metadata: {printed}, User ID: {user_id}"),
        status_code,
        headers,
    }
}

fn headers(fields: &Map<String, Value>) -> Headers {
    fields
        .iter()
        .filter_map(|(name, value)| value.as_str().map(|text| (name.clone(), text.to_owned())))
        .collect()
}

fn unexpected(chunk: &Value) -> OpenRouterError {
    let printed = serde_json::to_string(chunk).unwrap_or_else(|_| String::from("null"));
    OpenRouterError {
        message: format!("Got unexpected response from OpenRouter: {printed}"),
        status_code: 500,
        headers: Headers::new(),
    }
}
