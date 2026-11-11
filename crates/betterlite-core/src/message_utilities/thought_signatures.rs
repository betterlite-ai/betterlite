use betterlite_models::decisions::Json;
use thiserror::Error;

use super::json;

const GEMINI: &str = "gemini";
const GEMINI_PROVIDERS: [&str; 3] = ["gemini", "vertex_ai", "vertex_ai_beta"];
const VERTEX_PROVIDERS: [&str; 2] = ["vertex_ai", "vertex_ai_beta"];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThoughtSignatureError {
    #[error("empty separator")]
    EmptySeparator,
}

#[must_use]
pub fn is_gemini_model(model: Option<&str>, custom_llm_provider: Option<&str>) -> bool {
    if let Some(provider) = custom_llm_provider
        && GEMINI_PROVIDERS.contains(&provider)
    {
        if !VERTEX_PROVIDERS.contains(&provider) {
            return true;
        }
        return model.is_some_and(|name| name.to_lowercase().contains(GEMINI));
    }
    model.is_some_and(|name| name.to_lowercase().contains(GEMINI))
}

pub fn strip_signature_from_id(
    tool_call_id: &str,
    separator: &str,
) -> Result<String, ThoughtSignatureError> {
    if separator.is_empty() {
        return Err(ThoughtSignatureError::EmptySeparator);
    }
    Ok(match tool_call_id.split_once(separator) {
        Some((head, _signature)) => head.to_owned(),
        None => tool_call_id.to_owned(),
    })
}

pub fn strip_assistant_tool_calls(
    message: &Json,
    separator: &str,
) -> Result<Json, ThoughtSignatureError> {
    let Some(fields) = json::object(message)
        .filter(|fields| fields.get("role").and_then(json::text) == Some("assistant"))
    else {
        return Ok(message.clone());
    };
    let Some(Json::List(tool_calls)) = fields.get("tool_calls") else {
        return Ok(message.clone());
    };
    let mut updated = Vec::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        let Some(call_fields) = json::object(tool_call) else {
            updated.push(tool_call.clone());
            continue;
        };
        let mut cleaned_call = call_fields.clone();
        if let Some(id) = cleaned_call.get("id").and_then(json::text)
            && id.contains(separator)
        {
            let cleaned = strip_signature_from_id(id, separator)?;
            cleaned_call.insert(String::from("id"), Json::Text(cleaned));
        }
        updated.push(Json::Object(cleaned_call));
    }
    let mut processed_fields = fields.clone();
    processed_fields.insert(String::from("tool_calls"), Json::List(updated));
    Ok(Json::Object(processed_fields))
}

pub fn process_tool_message_id(
    message: &Json,
    separator: &str,
) -> Result<Json, ThoughtSignatureError> {
    let Some(tool_call_id) = json::object(message)
        .filter(|fields| fields.get("role").and_then(json::text) == Some("tool"))
        .and_then(|fields| fields.get("tool_call_id"))
        .and_then(json::text)
    else {
        return Ok(message.clone());
    };
    if !tool_call_id.contains(separator) {
        return Ok(message.clone());
    }
    let cleaned = strip_signature_from_id(tool_call_id, separator)?;
    let mut processed = json::object(message).cloned().unwrap_or_default();
    processed.insert(String::from("tool_call_id"), Json::Text(cleaned));
    Ok(Json::Object(processed))
}

pub fn strip_signatures_from_messages(
    messages: &[Json],
    separator: &str,
) -> Result<Vec<Json>, ThoughtSignatureError> {
    let mut processed = Vec::with_capacity(messages.len());
    for message in messages {
        if json::object(message).is_none() {
            processed.push(message.clone());
            continue;
        }
        let with_tool_calls = strip_assistant_tool_calls(message, separator)?;
        processed.push(process_tool_message_id(&with_tool_calls, separator)?);
    }
    Ok(processed)
}
