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
    let Some(fields) = json::object(message) else {
        return Ok(message.clone());
    };
    let is_assistant = fields.get("role").and_then(json::text) == Some("assistant");
    let Some(Json::List(tool_calls)) = fields.get("tool_calls").filter(|_| is_assistant) else {
        return Ok(message.clone());
    };
    let mut updated = Vec::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        let mut cleaned_call = tool_call.clone();
        if let Json::Object(call_fields) = &mut cleaned_call
            && let Some(id) = call_fields.get("id").and_then(json::text)
            && id.contains(separator)
        {
            let cleaned = strip_signature_from_id(id, separator)?;
            call_fields.insert(String::from("id"), Json::Text(cleaned));
        }
        updated.push(cleaned_call);
    }
    let mut processed_fields = fields.clone();
    processed_fields.insert(String::from("tool_calls"), Json::List(updated));
    Ok(Json::Object(processed_fields))
}

pub fn process_tool_message_id(
    message: &Json,
    separator: &str,
) -> Result<Json, ThoughtSignatureError> {
    let mut processed = message.clone();
    if let Json::Object(fields) = &mut processed
        && fields.get("role").and_then(json::text) == Some("tool")
        && let Some(tool_call_id) = fields.get("tool_call_id").and_then(json::text)
        && tool_call_id.contains(separator)
    {
        let cleaned = strip_signature_from_id(tool_call_id, separator)?;
        fields.insert(String::from("tool_call_id"), Json::Text(cleaned));
    }
    Ok(processed)
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
