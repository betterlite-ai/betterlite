use betterlite_a2a::a2a_text::{A2aResponse, extract_text_from_a2a_response};
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::agentcore_response::content::{AgentCoreContent, AgentCoreMessage};
use crate::agentcore_response::error::AgentCoreResponseError;
use crate::agentcore_response::reply::ParsedReply;

const A2A_MAX_DEPTH: usize = 10;

fn raw_reply(value: &Json) -> ParsedReply {
    ParsedReply {
        content: dumps(value),
        usage: None,
        final_message: None,
    }
}

pub fn parse_json_reply(value: &Json) -> Result<ParsedReply, AgentCoreResponseError> {
    let Json::Object(object) = value else {
        tracing::warn!("AgentCore: JSON response is not a dict. Returning raw JSON as content.");
        return Ok(raw_reply(value));
    };
    if let Some(content) = a2a_content(value, object)? {
        return Ok(ParsedReply {
            content,
            usage: None,
            final_message: None,
        });
    }
    if let Some(Json::Object(result)) = object.get("result") {
        let message = AgentCoreMessage::from_object(result);
        return Ok(ParsedReply {
            content: message.content.text()?,
            usage: None,
            final_message: Some(message),
        });
    }
    if let Some(Json::List(items)) = object.get("response") {
        return Ok(ParsedReply {
            content: AgentCoreContent::from_items(items).text()?,
            usage: None,
            final_message: None,
        });
    }
    if let Some(text) = first_text(object) {
        return Ok(ParsedReply {
            content: text,
            usage: None,
            final_message: None,
        });
    }
    tracing::warn!(
        "AgentCore: Could not extract content from JSON response keys {:?}. Returning raw JSON as content.",
        object.keys().collect::<Vec<_>>()
    );
    Ok(raw_reply(value))
}

fn first_text(object: &IndexMap<String, Json>) -> Option<String> {
    ["result", "response"]
        .into_iter()
        .find_map(|key| match object.get(key) {
            Some(Json::Text(text)) => Some(text.clone()),
            None | Some(_) => None,
        })
}

fn a2a_content(
    value: &Json,
    object: &IndexMap<String, Json>,
) -> Result<Option<String>, AgentCoreResponseError> {
    if !object.contains_key("jsonrpc") {
        return Ok(None);
    }
    let shape =
        serde_json::to_value(value).map_err(|source| AgentCoreResponseError::Encode { source })?;
    let a2a: A2aResponse = serde_json::from_value(shape)
        .map_err(|source| AgentCoreResponseError::A2aShape { source })?;
    let content = extract_text_from_a2a_response(&a2a, A2A_MAX_DEPTH)
        .map_err(|source| AgentCoreResponseError::A2aText { source })?;
    Ok((!content.is_empty()).then_some(content))
}
