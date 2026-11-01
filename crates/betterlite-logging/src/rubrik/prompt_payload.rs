use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use crate::rubrik::compact_json::compact_json;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FlattenedModerationMessage {
    pub role: Option<String>,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PromptModerationPayload {
    pub model: String,
    pub messages: Vec<FlattenedModerationMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_key: Option<String>,
}

pub struct PromptModerationInputs<'input> {
    pub model: Option<&'input str>,
    pub structured_messages: &'input [Json],
    pub tools: Option<Json>,
}

pub struct PromptModerationRequest<'input> {
    pub model: Option<&'input str>,
    pub user: Option<&'input str>,
    pub correlation_key: Option<&'input str>,
    pub litellm_call_id: Option<&'input str>,
}

#[must_use]
pub fn build_prompt_moderation(
    inputs: &PromptModerationInputs<'_>,
    request: &PromptModerationRequest<'_>,
) -> PromptModerationPayload {
    // request_data never carries "correlation_key" in practice; litellm_call_id is the stable join key the response/tool path uses (see request_data::correlation_id).
    let correlation_key = request
        .correlation_key
        .and_then(non_empty)
        .or_else(|| request.litellm_call_id.and_then(non_empty));
    PromptModerationPayload {
        model: inputs
            .model
            .and_then(non_empty)
            .or_else(|| request.model.and_then(non_empty))
            .unwrap_or("")
            .to_owned(),
        messages: flatten_moderation_messages(inputs.structured_messages),
        tools: inputs.tools.clone(),
        user: request.user.and_then(non_empty).map(str::to_owned),
        correlation_key: correlation_key.map(str::to_owned),
    }
}

#[must_use]
pub fn flatten_moderation_messages(messages: &[Json]) -> Vec<FlattenedModerationMessage> {
    let mut flattened = Vec::with_capacity(messages.len());
    for message in messages {
        if let Some(flat) = flatten_message(message) {
            flattened.push(flat);
        }
    }
    flattened
}

fn flatten_message(message: &Json) -> Option<FlattenedModerationMessage> {
    let Json::Object(fields) = message else {
        return None;
    };
    let role = match fields.get("role") {
        Some(Json::Text(role)) => Some(role.clone()),
        _ => None,
    };
    let content = moderation_text_parts(fields)
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Some(FlattenedModerationMessage { role, content })
}

// Every attacker-controlled text segment of a message: its content plus the arguments of any tool call or the deprecated single function call.
#[must_use]
pub fn moderation_text_parts(fields: &IndexMap<String, Json>) -> Vec<String> {
    let mut parts = vec![convert_content_list_to_str(fields)];
    if let Some(Json::List(tool_calls)) = fields.get("tool_calls") {
        for tool_call in tool_calls {
            if let Json::Object(tool_call_fields) = tool_call {
                parts.push(tool_call_arguments(tool_call_fields));
            }
        }
    }
    parts.push(function_call_arguments(fields.get("function_call")));
    parts
}

fn tool_call_arguments(tool_call_fields: &IndexMap<String, Json>) -> String {
    match tool_call_fields.get("function") {
        Some(Json::Object(function_fields)) => text_segment(function_fields.get("arguments")),
        _ => String::new(),
    }
}

fn function_call_arguments(function_call: Option<&Json>) -> String {
    match function_call {
        Some(Json::Object(fields)) => text_segment(fields.get("arguments")),
        _ => String::new(),
    }
}

fn text_segment(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => String::new(),
        Some(Json::Text(text)) => text.clone(),
        Some(other) => compact_json(other),
    }
}

// Flattens Anthropic-shaped content-part arrays to plain text; images are skipped.
fn convert_content_list_to_str(fields: &IndexMap<String, Json>) -> String {
    let mut texts = String::new();
    match fields.get("content") {
        Some(Json::List(parts)) => {
            for part in parts {
                if let Json::Object(part_fields) = part
                    && let Some(Json::Text(text)) = part_fields.get("text")
                {
                    texts.push_str(text);
                }
            }
        }
        Some(Json::Text(text)) => texts.push_str(text),
        _ => {}
    }
    texts.push_str(&extract_search_results_text(fields.get("search_results")));
    texts
}

// Counts every string field forwarded on an OpenAI tool-message search_results block.
fn extract_search_results_text(search_results: Option<&Json>) -> String {
    let Some(Json::List(results)) = search_results else {
        return String::new();
    };
    let mut texts = String::new();
    for result in results {
        let Json::Object(fields) = result else {
            continue;
        };
        for key in ["source", "title"] {
            if let Some(Json::Text(text)) = fields.get(key) {
                texts.push_str(text);
            }
        }
        if let Some(Json::List(blocks)) = fields.get("content") {
            texts.push_str(&extract_block_texts(blocks));
        }
        if let Some(citations) = fields.get("citations")
            && !matches!(citations, Json::Null)
        {
            texts.push_str(&compact_json(citations));
        }
    }
    texts
}

const fn non_empty(text: &str) -> Option<&str> {
    if text.is_empty() { None } else { Some(text) }
}

fn extract_block_texts(blocks: &[Json]) -> String {
    let mut texts = String::new();
    for block in blocks {
        if let Json::Object(block_fields) = block
            && let Some(Json::Text(text)) = block_fields.get("text")
        {
            texts.push_str(text);
        }
    }
    texts
}
