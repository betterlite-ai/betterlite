use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::agentcore_response::content::AgentCoreMessage;
use crate::agentcore_response::error::AgentCoreResponseError;
use crate::agentcore_response::reply::ParsedReply;
use crate::agentcore_response::usage::AgentCoreUsage;

pub fn parse_sse_stream(text: &str) -> Result<ParsedReply, AgentCoreResponseError> {
    let mut final_message: Option<AgentCoreMessage> = None;
    let mut usage: Option<AgentCoreUsage> = None;
    let mut deltas = String::new();
    for raw_line in text.trim().lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(Json::Object(data)) = extract_sse_json(line) else {
            continue;
        };
        if let Some(Json::Object(message)) = data.get("message") {
            final_message = Some(AgentCoreMessage::from_object(message));
        }
        if let Some(Json::Object(event)) = data.get("event") {
            if let Some(found) = usage_from_event(event)? {
                usage = Some(found);
            }
            if let Some(delta) = content_delta(event)? {
                deltas.push_str(&delta);
            }
        }
    }
    let content = match &final_message {
        Some(message) if !message.is_empty => message.content.text()?,
        _ => deltas,
    };
    Ok(ParsedReply {
        content,
        usage,
        final_message,
    })
}

fn extract_sse_json(line: &str) -> Option<Json> {
    let after_prefix = line.strip_prefix("data:")?;
    let data_text = after_prefix.trim();
    if data_text.is_empty() {
        return None;
    }
    let Ok(data) = serde_json::from_str::<Json>(data_text) else {
        tracing::debug!("Skipping non-JSON line");
        return None;
    };
    Some(data)
}

fn object_field<'value>(
    object: &'value IndexMap<String, Json>,
    key: &str,
    not_object: AgentCoreResponseError,
) -> Result<Option<&'value IndexMap<String, Json>>, AgentCoreResponseError> {
    match object.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => Err(not_object),
    }
}

fn usage_from_event(
    event: &IndexMap<String, Json>,
) -> Result<Option<AgentCoreUsage>, AgentCoreResponseError> {
    let Some(metadata_fields) =
        object_field(event, "metadata", AgentCoreResponseError::MetadataNotObject)?
    else {
        return Ok(None);
    };
    let Some(usage) = metadata_fields.get("usage") else {
        return Ok(None);
    };
    AgentCoreUsage::from_value(usage)
}

fn content_delta(event: &IndexMap<String, Json>) -> Result<Option<String>, AgentCoreResponseError> {
    let Some(block_fields) = object_field(
        event,
        "contentBlockDelta",
        AgentCoreResponseError::DeltaEventNotObject,
    )?
    else {
        return Ok(None);
    };
    let Some(delta_fields) = object_field(
        block_fields,
        "delta",
        AgentCoreResponseError::DeltaNotObject,
    )?
    else {
        return Ok(None);
    };
    match delta_fields.get("text") {
        None | Some(Json::Null) => Ok(None),
        // concatenating an empty delta text changes nothing, so there is no separate empty branch
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AgentCoreResponseError::DeltaTextNotString)
        }
    }
}
