use indexmap::IndexMap;
use serde::Serialize;

mod provider_fields;
pub mod wire;

use betterlite_models::decisions::Json;
use wire::{AnthropicCompletionResponse, AnthropicToolCall};
use wire::{JsonFields, get, json_text};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnthropicContentReply {
    pub content: String,
    pub citations: Option<Vec<Json>>,
    pub thinking_blocks: Option<Vec<Json>>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<AnthropicToolCall>,
    pub web_search_results: Option<Vec<Json>>,
    pub tool_results: Option<Vec<Json>>,
    pub compaction_blocks: Option<Vec<Json>>,
    pub extra_content: Option<String>,
    pub provider_specific_fields: JsonFields,
}

#[derive(Debug, thiserror::Error)]
pub enum ResponseContentError {
    #[error("invalid JSON tool arguments: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

pub fn decode_json_mode_arguments(arguments: &str) -> Result<Json, ResponseContentError> {
    match serde_json::from_str(arguments) {
        Ok(value) => Ok(value),
        Err(error) => Err(ResponseContentError::from(error)),
    }
}

pub fn project_response(
    response: &AnthropicCompletionResponse,
    json_mode: Option<bool>,
) -> Result<AnthropicContentReply, ResponseContentError> {
    let mut reply = AnthropicContentReply {
        content: String::new(),
        citations: None,
        thinking_blocks: None,
        reasoning_content: None,
        tool_calls: Vec::new(),
        web_search_results: None,
        tool_results: None,
        compaction_blocks: None,
        extra_content: None,
        provider_specific_fields: IndexMap::new(),
    };
    collect_content(response, &mut reply)?;
    resolve_json_mode(&mut reply, json_mode)?;
    reply.provider_specific_fields = provider_fields::project(response, &reply);
    Ok(reply)
}

fn collect_content(
    response: &AnthropicCompletionResponse,
    reply: &mut AnthropicContentReply,
) -> Result<(), ResponseContentError> {
    for (index, block) in response.content.iter().enumerate() {
        collect_block(block, index, reply)?;
    }
    Ok(())
}

fn collect_block(
    block: &Json,
    index: usize,
    reply: &mut AnthropicContentReply,
) -> Result<(), ResponseContentError> {
    match get(block, "type").and_then(json_text).unwrap_or_default() {
        "text" => collect_text(block, reply),
        "tool_use" | "server_tool_use" => {
            reply
                .tool_calls
                .push(wire::convert_tool_block(block, index)?);
        }
        "thinking" | "redacted_thinking" => collect_thinking(block, reply),
        "compaction" => reply
            .compaction_blocks
            .get_or_insert_with(Vec::new)
            .push(block.clone()),
        "tool_search_tool_result" => return Ok(()),
        kind if kind.ends_with("_tool_result") => collect_tool_result(kind, block, reply),
        _ => {}
    }
    collect_citations(block, reply);
    Ok(())
}

fn collect_text(block: &Json, reply: &mut AnthropicContentReply) {
    if let Some(text) = get(block, "text").and_then(json_text) {
        reply.content.push_str(text);
    }
}

fn collect_thinking(block: &Json, reply: &mut AnthropicContentReply) {
    reply
        .thinking_blocks
        .get_or_insert_with(Vec::new)
        .push(block.clone());
    if let Some(thinking) = get(block, "thinking").and_then(json_text) {
        reply
            .reasoning_content
            .get_or_insert_with(String::new)
            .push_str(thinking);
    } else {
        reply.reasoning_content.get_or_insert_with(String::new);
    }
}

fn collect_tool_result(kind: &str, block: &Json, reply: &mut AnthropicContentReply) {
    if matches!(kind, "web_search_tool_result" | "web_fetch_tool_result") {
        reply
            .web_search_results
            .get_or_insert_with(Vec::new)
            .push(block.clone());
    } else {
        reply
            .tool_results
            .get_or_insert_with(Vec::new)
            .push(block.clone());
    }
}

fn collect_citations(block: &Json, reply: &mut AnthropicContentReply) {
    let Some(Json::List(citations)) =
        get(block, "citations").filter(|value| !matches!(value, Json::Null))
    else {
        return;
    };
    let text = get(block, "text").and_then(json_text).unwrap_or_default();
    let adjusted = citations
        .iter()
        .map(|citation| citation_with_text(citation, text))
        .collect();
    reply
        .citations
        .get_or_insert_with(Vec::new)
        .push(Json::List(adjusted));
}

fn citation_with_text(citation: &Json, text: &str) -> Json {
    let mut fields = match citation {
        Json::Object(fields) => fields.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            IndexMap::new()
        }
    };
    fields.insert("supported_text".to_owned(), Json::Text(text.to_owned()));
    Json::Object(fields)
}

fn resolve_json_mode(
    reply: &mut AnthropicContentReply,
    json_mode: Option<bool>,
) -> Result<(), ResponseContentError> {
    if json_mode != Some(true) {
        return Ok(());
    }
    let internal = reply
        .tool_calls
        .iter()
        .enumerate()
        .filter_map(|(index, call)| {
            (call.function.name.as_deref() == Some("json_tool_call")).then_some(index)
        })
        .collect::<Vec<_>>();
    let Some(first) = internal
        .first()
        .and_then(|index| reply.tool_calls.get(*index))
    else {
        return Ok(());
    };
    if internal.len() == reply.tool_calls.len() {
        if let Some(arguments) = first.function.arguments.as_deref() {
            reply.extra_content = Some(json_tool_content(arguments)?);
            reply.tool_calls.clear();
        }
        return Ok(());
    }
    if let Some(arguments) = first.function.arguments.as_deref() {
        reply.extra_content = Some(json_tool_content(arguments)?);
    }
    reply.tool_calls = reply
        .tool_calls
        .iter()
        .enumerate()
        .filter(|(index, _)| !internal.contains(index))
        .map(|(_, call)| call.clone())
        .collect();
    Ok(())
}

fn json_tool_content(arguments: &str) -> Result<String, ResponseContentError> {
    let Ok(parsed) = decode_json_mode_arguments(arguments) else {
        return Ok(arguments.to_owned());
    };
    if let Json::Object(fields) = &parsed
        && let Some(value) = fields
            .get("values")
            .filter(|value| !matches!(value, Json::Null))
    {
        return serde_json::to_string(value).map_err(ResponseContentError::from);
    }
    serde_json::to_string(&parsed).map_err(ResponseContentError::from)
}

mod tests;
