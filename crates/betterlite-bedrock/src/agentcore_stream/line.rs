use std::str;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

use crate::agentcore_response::usage::AgentCoreUsage;

pub const MAX_LINE_BYTES: usize = 0x0010_0000;

const DATA_PREFIX: &str = "data:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineEvent {
    Content(String),
    Usage(AgentCoreUsage),
    MessageStop,
}

#[derive(Debug, Error)]
pub enum LineError {
    #[error("the AgentCore stream line is longer than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("event contentBlockDelta must be an object")]
    DeltaEventNotObject,
    #[error("event contentBlockDelta.delta must be an object")]
    DeltaNotObject,
    #[error("event contentBlockDelta.delta.text must be a string")]
    DeltaTextNotString,
    #[error("event metadata must be an object")]
    MetadataNotObject,
    #[error("event metadata usage must be an object")]
    UsageNotObject,
    #[error("event metadata usage field {field} must be an integer")]
    UsageNotInteger { field: &'static str },
    #[error("event metadata usage field {field} cannot be negative")]
    UsageNegative { field: &'static str },
}

pub fn decode_agentcore_line(
    buffer: &mut Vec<u8>,
    bytes: &[u8],
) -> Result<Vec<LineEvent>, LineError> {
    if bytes.len() > MAX_LINE_BYTES && !bytes.contains(&b'\n') {
        return Err(LineError::TooLarge {
            limit: MAX_LINE_BYTES,
        });
    }
    buffer.extend_from_slice(bytes);
    let mut events = Vec::new();
    let mut start = 0_usize;
    while let Some(offset) = buffer
        .get(start..)
        .and_then(|rest| rest.iter().position(|byte| *byte == b'\n'))
    {
        let end = start.checked_add(offset).ok_or(LineError::TooLarge {
            limit: MAX_LINE_BYTES,
        })?;
        if let Some(line) = buffer.get(start..end) {
            decode_line(line, &mut events)?;
        }
        start = end.checked_add(1).ok_or(LineError::TooLarge {
            limit: MAX_LINE_BYTES,
        })?;
    }
    buffer.drain(..start);
    if buffer.len() > MAX_LINE_BYTES {
        return Err(LineError::TooLarge {
            limit: MAX_LINE_BYTES,
        });
    }
    Ok(events)
}

fn decode_line(raw: &[u8], events: &mut Vec<LineEvent>) -> Result<(), LineError> {
    let Ok(text) = str::from_utf8(raw) else {
        tracing::debug!("Skipping non-JSON SSE line");
        return Ok(());
    };
    let Some(rest) = text.trim().strip_prefix(DATA_PREFIX) else {
        return Ok(());
    };
    let payload = rest.trim();
    if payload.is_empty() {
        return Ok(());
    }
    let Ok(parsed) = serde_json::from_str::<Json>(payload) else {
        tracing::debug!("Skipping non-JSON SSE line");
        return Ok(());
    };
    let Json::Object(data) = parsed else {
        return Ok(());
    };
    if let Some(Json::Object(event)) = data.get("event") {
        content_delta(event, events)?;
        usage_event(event, events)?;
    }
    if let Some(Json::Object(_)) = data.get("message") {
        events.push(LineEvent::MessageStop);
    }
    Ok(())
}

fn content_delta(
    event: &IndexMap<String, Json>,
    events: &mut Vec<LineEvent>,
) -> Result<(), LineError> {
    match event.get("contentBlockDelta") {
        None | Some(Json::Null) => Ok(()),
        Some(Json::Object(block)) => text_delta(block, events),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(LineError::DeltaEventNotObject)
        }
    }
}

fn text_delta(
    block: &IndexMap<String, Json>,
    events: &mut Vec<LineEvent>,
) -> Result<(), LineError> {
    match block.get("delta") {
        None => Ok(()),
        Some(Json::Object(delta)) => match delta.get("text") {
            None | Some(Json::Null) => Ok(()),
            Some(Json::Text(text)) if text.is_empty() => Ok(()),
            Some(Json::Text(text)) => {
                events.push(LineEvent::Content(text.clone()));
                Ok(())
            }
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                Err(LineError::DeltaTextNotString)
            }
        },
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(LineError::DeltaNotObject)
        }
    }
}

fn usage_event(
    event: &IndexMap<String, Json>,
    events: &mut Vec<LineEvent>,
) -> Result<(), LineError> {
    let reported = match event.get("metadata") {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::Object(metadata)) => metadata.get("usage"),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(LineError::MetadataNotObject);
        }
    };
    let Some(usage) = reported else {
        return Ok(());
    };
    let Json::Object(fields) = usage else {
        return Err(LineError::UsageNotObject);
    };
    events.push(LineEvent::Usage(AgentCoreUsage {
        input_tokens: token(fields, "inputTokens")?,
        output_tokens: token(fields, "outputTokens")?,
        total_tokens: token(fields, "totalTokens")?,
    }));
    Ok(())
}

fn token(fields: &IndexMap<String, Json>, field: &'static str) -> Result<u64, LineError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => integer(number, field),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(LineError::UsageNotInteger { field })
        }
    }
}

fn integer(number: &Number, field: &'static str) -> Result<u64, LineError> {
    if let Some(count) = number.as_u64() {
        return Ok(count);
    }
    number
        .as_i64()
        .map_or(Err(LineError::UsageNotInteger { field }), |negative| {
            u64::try_from(negative).map_err(|_source| LineError::UsageNegative { field })
        })
}
