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

#[derive(Clone, Copy)]
pub(crate) enum ChunkShape<'text> {
    Content(Option<&'text str>),
    Final,
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
    if let Json::List(items) = data {
        return process_event(items, model, ids);
    }
    let Some(fields) = data.as_object() else {
        return Ok(LineOutcome::Skipped);
    };
    process_object(fields, model, ids)
}

// a short list comes back skipped, since LiteLLM's dict branch is an elif of the list check
fn process_event(
    items: &[Json],
    model: &str,
    ids: &dyn IdSource,
) -> Result<LineOutcome, LangGraphStreamError> {
    let Some((event_type, payload)) = split_event(items) else {
        return Ok(LineOutcome::Skipped);
    };
    if event_type == "metadata" {
        let has_run_id = payload
            .as_object()
            .is_some_and(|fields| fields.contains_key("run_id"));
        return Ok(if has_run_id {
            LineOutcome::Final(make_chunk(model, ids, ChunkShape::Final))
        } else {
            LineOutcome::Skipped
        });
    }
    let Json::List(entries) = payload else {
        return Ok(LineOutcome::Skipped);
    };
    for item in entries {
        let message = match item {
            Json::List(entry) => entry.first().and_then(Json::as_object),
            Json::Object(fields) => Some(fields),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => None,
        };
        let Some(found) = message else {
            continue;
        };
        if event_type == "messages"
            && let Some(text) = ai_text(found)?
        {
            return Ok(LineOutcome::Chunk(make_chunk(
                model,
                ids,
                ChunkShape::Content(Some(text)),
            )));
        }
    }
    Ok(LineOutcome::Skipped)
}

fn split_event(items: &[Json]) -> Option<(&str, &Json)> {
    let [event_type, payload, ..] = items else {
        return None;
    };
    Some((event_type.as_str()?, payload))
}

// an empty or null content is not an error: LiteLLM's condition falls through to the next item
fn ai_text(message: &Object) -> Result<Option<&str>, LangGraphStreamError> {
    let kind = message.get("type").and_then(Json::as_str);
    if !matches!(kind, Some("ai" | "AIMessageChunk")) {
        return Ok(None);
    }
    let Some(content) = message.get("content") else {
        return Ok(None);
    };
    Ok(text_or_null(content)?.filter(|text| !text.is_empty()))
}

fn text_or_null(value: &Json) -> Result<Option<&str>, LangGraphStreamError> {
    match value {
        Json::Text(text) => Ok(Some(text)),
        Json::Null => Ok(None),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
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
        let text = text_or_null(content)?;
        return Ok(LineOutcome::Chunk(make_chunk(
            model,
            ids,
            ChunkShape::Content(text),
        )));
    }
    let messages = match fields.get("messages") {
        None | Some(Json::Null) => return Ok(LineOutcome::Skipped),
        Some(Json::List(messages)) => messages,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(LangGraphStreamError::FieldType { field: "messages" });
        }
    };
    let Some(Json::Object(last)) = messages.last() else {
        return Ok(LineOutcome::Skipped);
    };
    if last.get("type").and_then(Json::as_str) != Some("ai") {
        return Ok(LineOutcome::Skipped);
    }
    let text = match last.get("content") {
        None => Some(""),
        Some(value) => text_or_null(value)?,
    };
    Ok(LineOutcome::Chunk(make_chunk(
        model,
        ids,
        ChunkShape::Content(text),
    )))
}

pub(crate) fn make_chunk(model: &str, ids: &dyn IdSource, shape: ChunkShape<'_>) -> Chunk {
    let delta = match shape {
        ChunkShape::Content(text) => Delta {
            role: Some(CONTENT_ROLE.to_owned()),
            content: text.map(str::to_owned),
            refusal: None,
            tool_calls: None,
            function_call: None,
        },
        ChunkShape::Final => Delta::default(),
    };
    Chunk {
        id: format!("{CHAT_COMPLETION_ID_PREFIX}{}", ids.next_id()),
        object: CHUNK_OBJECT.to_owned(),
        created: 0,
        model: model.to_owned(),
        choices: vec![Choice {
            index: 0,
            delta,
            finish_reason: match shape {
                ChunkShape::Content(_) => None,
                ChunkShape::Final => Some(FINISH_STOP.to_owned()),
            },
            logprobs: None,
        }],
        usage: None,
        system_fingerprint: None,
        service_tier: None,
    }
}
