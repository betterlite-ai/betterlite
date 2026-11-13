use crate::arguments::json::{Json, Object};
use crate::message_content::{Message, get_str_from_messages};
use thiserror::Error;

const MAX_RESPONSE_BYTES: usize = 0x0010_0000;
const MAX_RESPONSES_DEPTH: usize = 64;

#[derive(Debug, Error)]
pub enum SemanticCacheRecordsError {
    #[error("Responses input exceeds the maximum nesting depth")]
    ResponsesDepthExceeded,
    #[error("cached response payload is {actual} bytes; maximum is {maximum}")]
    PayloadTooLarge { actual: usize, maximum: usize },
    #[error("cached response is not valid JSON")]
    Json(#[source] serde_json::Error),
}

pub fn extract_semantic_prompt(
    messages: Option<&[Message]>,
    responses_input: Option<&Json>,
) -> Result<Option<String>, SemanticCacheRecordsError> {
    if let Some(input_messages) = messages {
        return Ok(Some(get_str_from_messages(input_messages)));
    }
    let Some(input_value) = responses_input else {
        return Ok(None);
    };
    let mut parts = Vec::new();
    collect_responses_text(input_value, 0, &mut parts)?;
    if parts.is_empty() {
        return Ok(None);
    }
    Ok(Some(parts.join("\n").trim().to_owned()))
}

fn collect_responses_text(
    value: &Json,
    depth: usize,
    parts: &mut Vec<String>,
) -> Result<(), SemanticCacheRecordsError> {
    let container_depth = depth
        .checked_add(1)
        .ok_or(SemanticCacheRecordsError::ResponsesDepthExceeded)?;
    if matches!(value, Json::List(_) | Json::Object(_)) && container_depth > MAX_RESPONSES_DEPTH {
        return Err(SemanticCacheRecordsError::ResponsesDepthExceeded);
    }
    match value {
        Json::Text(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_owned());
            }
        }
        Json::List(items) => {
            for item in items {
                collect_responses_text(item, container_depth, parts)?;
            }
        }
        Json::Object(fields) => {
            match fields
                .get("content")
                .filter(|content| !matches!(content, Json::Null))
            {
                Some(content) => {
                    collect_responses_text(content, container_depth, parts)?;
                }
                None => {
                    if let Some(text) = first_text_field(fields) {
                        parts.push(text.to_owned());
                    }
                }
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) => {}
    }
    Ok(())
}

fn first_text_field(fields: &Object) -> Option<&str> {
    for key in ["text", "output", "input_text", "output_text"] {
        if let Some(text) = fields.get(key).and_then(Json::as_str).map(str::trim)
            && !text.is_empty()
        {
            return Some(text);
        }
    }
    None
}

pub fn decode_cached_response(
    payload: Option<&[u8]>,
) -> Result<Option<Json>, SemanticCacheRecordsError> {
    let Some(bytes) = payload else {
        return Ok(None);
    };
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(SemanticCacheRecordsError::PayloadTooLarge {
            actual: bytes.len(),
            maximum: MAX_RESPONSE_BYTES,
        });
    }
    serde_json::from_slice(bytes)
        .map(Some)
        .map_err(SemanticCacheRecordsError::Json)
}

mod tests;
