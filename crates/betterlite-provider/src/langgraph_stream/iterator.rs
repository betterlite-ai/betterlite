use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta};
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::LangGraphStreamError;
use super::{CHUNK_OBJECT, CONTENT_ROLE, DATA_PREFIX, FINISH_STOP, IdSource, MAX_LOGGED_LINE};

#[derive(Debug)]
pub enum LineOutcome {
    Chunk(Chunk),
    Final(Chunk),
    Skipped,
}

pub fn parse_sse_line(
    line: &str,
    model: &str,
    ids: &dyn IdSource,
) -> Result<LineOutcome, LangGraphStreamError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(LineOutcome::Skipped);
    }
    let Some(rest) = trimmed.strip_prefix(DATA_PREFIX) else {
        return Ok(LineOutcome::Skipped);
    };
    let payload = rest.trim();
    if payload.is_empty() {
        return Ok(LineOutcome::Skipped);
    }
    let Ok(data) = serde_json::from_str::<Json>(payload) else {
        tracing::debug!(
            "Skipping non-JSON SSE line: {}",
            trimmed.chars().take(MAX_LOGGED_LINE).collect::<String>()
        );
        return Ok(LineOutcome::Skipped);
    };
    process_data(&data, model, ids)
}

fn process_data(
    data: &Json,
    model: &str,
    ids: &dyn IdSource,
) -> Result<LineOutcome, LangGraphStreamError> {
    match data {
        Json::List(items) => process_list(items, model, ids),
        Json::Object(fields) => process_object(fields, model, ids),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => Ok(LineOutcome::Skipped),
    }
}

// a list event of any other shape is not a dict event too: LiteLLM's dict branch is an elif of the list check
fn process_list(
    items: &[Json],
    model: &str,
    ids: &dyn IdSource,
) -> Result<LineOutcome, LangGraphStreamError> {
    let [event_type, payload, ..] = items else {
        return Ok(LineOutcome::Skipped);
    };
    match event_type.as_str() {
        Some("messages") => process_messages(payload, model, ids),
        Some("metadata") => Ok(process_metadata(payload, model, ids)),
        Some(_) | None => Ok(LineOutcome::Skipped),
    }
}

fn process_metadata(payload: &Json, model: &str, ids: &dyn IdSource) -> LineOutcome {
    match payload {
        Json::Object(fields) if fields.contains_key("run_id") => {
            LineOutcome::Final(final_chunk(model, ids))
        }
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => LineOutcome::Skipped,
    }
}

fn process_messages(
    payload: &Json,
    model: &str,
    ids: &dyn IdSource,
) -> Result<LineOutcome, LangGraphStreamError> {
    let Json::List(items) = payload else {
        return Ok(LineOutcome::Skipped);
    };
    for item in items {
        match item {
            Json::List(entry) => {
                let Some(Json::Object(message)) = entry.first() else {
                    continue;
                };
                if let Some(text) = ai_text(message)? {
                    return Ok(LineOutcome::Chunk(content_chunk(Some(text), model, ids)));
                }
            }
            Json::Object(message) => {
                if let Some(text) = ai_text(message)? {
                    return Ok(LineOutcome::Chunk(content_chunk(Some(text), model, ids)));
                }
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
        }
    }
    Ok(LineOutcome::Skipped)
}

// an empty or null content is not an error: LiteLLM's condition falls through to the next item
fn ai_text(message: &Object) -> Result<Option<&str>, LangGraphStreamError> {
    let kind = message.get("type").and_then(Json::as_str);
    if !matches!(kind, Some("ai" | "AIMessageChunk")) {
        return Ok(None);
    }
    match message.get("content") {
        Some(Json::Text(text)) if !text.is_empty() => Ok(Some(text)),
        Some(Json::Text(_) | Json::Null) | None => Ok(None),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(LangGraphStreamError::FieldType { field: "content" })
        }
    }
}

// the content key wins over messages, and a null content is a chunk with no text
fn process_object(
    fields: &Object,
    model: &str,
    ids: &dyn IdSource,
) -> Result<LineOutcome, LangGraphStreamError> {
    if let Some(content) = fields.get("content") {
        return object_content(content, model, ids);
    }
    match fields.get("messages") {
        None | Some(Json::Null) => Ok(LineOutcome::Skipped),
        Some(Json::List(messages)) => process_last_message(messages, model, ids),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(LangGraphStreamError::FieldType { field: "messages" })
        }
    }
}

fn object_content(
    content: &Json,
    model: &str,
    ids: &dyn IdSource,
) -> Result<LineOutcome, LangGraphStreamError> {
    match content {
        Json::Null => Ok(LineOutcome::Chunk(content_chunk(None, model, ids))),
        Json::Text(text) => Ok(LineOutcome::Chunk(content_chunk(Some(text), model, ids))),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(LangGraphStreamError::FieldType { field: "content" })
        }
    }
}

fn process_last_message(
    messages: &[Json],
    model: &str,
    ids: &dyn IdSource,
) -> Result<LineOutcome, LangGraphStreamError> {
    let Some(Json::Object(last)) = messages.last() else {
        return Ok(LineOutcome::Skipped);
    };
    if last.get("type").and_then(Json::as_str) != Some("ai") {
        return Ok(LineOutcome::Skipped);
    }
    let text = match last.get("content") {
        None => Some(""),
        Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(LangGraphStreamError::FieldType { field: "content" });
        }
    };
    Ok(LineOutcome::Chunk(content_chunk(text, model, ids)))
}

pub(crate) fn content_chunk(text: Option<&str>, model: &str, ids: &dyn IdSource) -> Chunk {
    Chunk {
        id: format!("{CHAT_COMPLETION_ID_PREFIX}{}", ids.next_id()),
        object: CHUNK_OBJECT.to_owned(),
        created: 0,
        model: model.to_owned(),
        choices: vec![Choice {
            index: 0,
            delta: Delta {
                role: Some(CONTENT_ROLE.to_owned()),
                content: text.map(str::to_owned),
                refusal: None,
                tool_calls: None,
                function_call: None,
            },
            finish_reason: None,
            logprobs: None,
        }],
        usage: None,
        system_fingerprint: None,
        service_tier: None,
    }
}

pub(crate) fn final_chunk(model: &str, ids: &dyn IdSource) -> Chunk {
    Chunk {
        id: format!("{CHAT_COMPLETION_ID_PREFIX}{}", ids.next_id()),
        object: CHUNK_OBJECT.to_owned(),
        created: 0,
        model: model.to_owned(),
        choices: vec![Choice {
            index: 0,
            delta: Delta {
                role: None,
                content: None,
                refusal: None,
                tool_calls: None,
                function_call: None,
            },
            finish_reason: Some(FINISH_STOP.to_owned()),
            logprobs: None,
        }],
        usage: None,
        system_fingerprint: None,
        service_tier: None,
    }
}
