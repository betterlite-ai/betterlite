use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::ToolResultsError;
use super::records::{
    CachedCallMessage, ContentBlock, FunctionChunk, ImageUrl, ResultMessage, ToolCallChunk,
    ToolContent, ToolMessage, text_field,
};
use crate::api::responses_codex_tools::custom_tools::serialize_tool_call_arguments;

fn image_url(part: &IndexMap<String, Json>) -> Option<&str> {
    let candidate = match part.get("image_url") {
        None | Some(Json::Null) => part.get("url"),
        Some(Json::Text(text)) if text.is_empty() => part.get("url"),
        Some(Json::Object(fields)) if fields.is_empty() => part.get("url"),
        found => found,
    };
    let url = match candidate {
        Some(Json::Object(fields)) => fields.get("url"),
        other => other,
    };
    match url {
        Some(Json::Text(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn list_content(original: &Json, parts: &[Json]) -> ToolContent {
    let mut blocks = Vec::new();
    let mut joined = String::new();
    let mut has_image = false;
    for part in parts {
        let Json::Object(fields) = part else {
            continue;
        };
        let Some(Json::Text(kind)) = fields.get("type") else {
            continue;
        };
        match kind.as_str() {
            "input_text" | "output_text" | "text" => {
                if let Some(Json::Text(text)) = fields.get("text")
                    && !text.is_empty()
                {
                    joined.push_str(text);
                    blocks.push(ContentBlock::Text { text: text.clone() });
                }
            }
            "input_image" | "image_url" => {
                if let Some(url) = image_url(fields) {
                    has_image = true;
                    blocks.push(ContentBlock::ImageUrl {
                        image_url: ImageUrl {
                            url: url.to_owned(),
                        },
                    });
                }
            }
            _ => {}
        }
    }
    if has_image {
        ToolContent::Blocks(blocks)
    } else if joined.is_empty() {
        ToolContent::Text(dumps(original))
    } else {
        ToolContent::Text(joined)
    }
}

#[must_use]
pub fn tool_output_content(output: Option<&Json>) -> ToolContent {
    match output {
        None | Some(Json::Null) => ToolContent::Text(String::new()),
        Some(Json::Text(text)) => ToolContent::Text(text.clone()),
        Some(original @ Json::List(parts)) => list_content(original, parts),
        Some(other) => ToolContent::Text(dumps(other)),
    }
}

fn cached_definition(
    cached: Option<CacheValue>,
) -> Result<Option<IndexMap<String, Json>>, ToolResultsError> {
    let Some(value) = cached else {
        return Ok(None);
    };
    let plain = serde_json::to_value(&value)
        .and_then(serde_json::from_value::<Json>)
        .map_err(|source| ToolResultsError::CachedNotJson { source })?;
    match plain {
        Json::Object(fields) if !fields.is_empty() => Ok(Some(fields)),
        Json::Null | Json::Object(_) => Ok(None),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ToolResultsError::CachedNotObject)
        }
    }
}

fn replayed_call(
    definition: &IndexMap<String, Json>,
) -> Result<CachedCallMessage, ToolResultsError> {
    let empty = IndexMap::new();
    let function = match definition.get("function") {
        None | Some(Json::Null) => &empty,
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(ToolResultsError::NotObject { field: "function" });
        }
    };
    let arguments = function.get("arguments").unwrap_or(&Json::Null);
    let call = ToolCallChunk {
        id: text_field(definition, "id", "")?.to_owned(),
        kind: text_field(definition, "type", "function")?.to_owned(),
        function: FunctionChunk {
            name: text_field(function, "name", "")?.to_owned(),
            arguments: serialize_tool_call_arguments(arguments, ""),
            provider_specific_fields: None,
        },
        index: 0,
        provider_specific_fields: None,
    };
    Ok(CachedCallMessage {
        role: "assistant",
        tool_calls: vec![call],
    })
}

// the cache is keyed by text ids, so a numeric call_id never matches an entry, as in Python
pub fn tool_call_output_messages(
    item: &IndexMap<String, Json>,
    cache: &DualLocalCache,
) -> Result<Vec<ResultMessage>, ToolResultsError> {
    let (tool_call_id, cache_key) = match item.get("call_id") {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::Text(id)) if id.is_empty() => return Ok(Vec::new()),
        Some(Json::Text(id)) => (id.clone(), Some(id.as_str())),
        Some(Json::Number(id)) if id.as_i64() == Some(0) => return Ok(Vec::new()),
        Some(Json::Number(id)) => (id.to_string(), None),
        Some(Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ToolResultsError::CallId);
        }
    };
    let tool_message = ResultMessage::Tool(ToolMessage {
        role: "tool",
        content: tool_output_content(item.get("output")),
        tool_call_id,
    });
    let Some(definition) = cached_definition(cache_key.and_then(|key| cache.get(key)))? else {
        return Ok(vec![tool_message]);
    };
    Ok(vec![
        ResultMessage::CachedCall(replayed_call(&definition)?),
        tool_message,
    ])
}
