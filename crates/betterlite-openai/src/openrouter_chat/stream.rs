use serde_json::{Map, Value};

use crate::chat_choices::wire::{ProviderField, StreamChoice, StreamReply};
use crate::openai_base_config::Headers;
use crate::openrouter_chat::error::{OpenRouterError, key_error};

pub const CHUNK_OBJECT: &str = "chat.completion.chunk";

// B010's iterator calls this for one parsed chunk; every missing key is LiteLLM's KeyError branch
pub fn chunk_parser(chunk: &Value) -> Result<StreamReply, OpenRouterError> {
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
    let mut choices = Vec::with_capacity(raw_choices.len());
    for raw in raw_choices {
        choices.push(choice(raw, chunk)?);
    }
    let id = fields
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error("id", chunk))?;
    let created = fields
        .get("created")
        .and_then(Value::as_i64)
        .ok_or_else(|| key_error("created", chunk))?;
    let model = fields
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error("model", chunk))?;
    Ok(StreamReply {
        id: id.to_owned(),
        object: CHUNK_OBJECT,
        created,
        model: Some(model.to_owned()),
        choices,
        usage: fields
            .get("usage")
            .cloned()
            .filter(|usage| !usage.is_null()),
        service_tier: None,
    })
}

fn choice(raw: &Value, chunk: &Value) -> Result<StreamChoice, OpenRouterError> {
    let delta = raw
        .as_object()
        .and_then(|fields| fields.get("delta"))
        .filter(|delta| delta.is_object())
        .ok_or_else(|| key_error("delta", chunk))?;
    let mut choice: StreamChoice =
        serde_json::from_value(raw.clone()).map_err(|_| unexpected(chunk))?;
    choice.delta.reasoning_content = match delta.get("reasoning") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Null) | None => None,
        Some(_) => return Err(unexpected(chunk)),
    };
    Ok(choice)
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
