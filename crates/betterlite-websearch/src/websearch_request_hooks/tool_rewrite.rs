use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use thiserror::Error;

use crate::tools::{
    LITELLM_WEB_SEARCH_TOOL_NAME, get_litellm_web_search_tool, openai_web_search_tool,
    responses_web_search_tool,
};

const FUNCTION: &str = "function";
const LEGACY_NAME: &str = "WebSearch";
const WEB_SEARCH: &str = "web_search";
const WEB_SEARCH_PREFIX: &str = "web_search_";

#[derive(Debug, Error)]
pub enum RewriteError {
    #[error("tools must be a list of tool objects")]
    Tools,
    #[error("tool {index} must be an object")]
    Tool { index: usize },
    #[error("tool {index} has a non-string {field} field")]
    Field { index: usize, field: &'static str },
    #[error("cannot encode the litellm_web_search tool")]
    Encode(#[from] serde_json::Error),
}

pub fn encode<T>(value: &T) -> Result<Json, RewriteError>
where
    T: Serialize,
{
    let text = serde_json::to_string(value)?;
    let parsed: Json = serde_json::from_str(&text)?;
    Ok(parsed)
}

fn text_field<'tool>(
    fields: &'tool Object,
    field: &'static str,
    index: usize,
) -> Result<Option<&'tool str>, RewriteError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(RewriteError::Field { index, field }),
    }
}

fn absent_if_null(value: Option<&Json>) -> Option<&Json> {
    match value {
        None | Some(Json::Null) => None,
        Some(other) => Some(other),
    }
}

// LiteLLM's _tool_name: the OpenAI function wrapper's name, else the tool's own name
fn effective_name(tool: &Json, index: usize) -> Result<Option<&Json>, RewriteError> {
    let Some(fields) = tool.as_object() else {
        return Err(RewriteError::Tool { index });
    };
    if text_field(fields, "type", index)? == Some(FUNCTION)
        && let Some(function) = fields.get(FUNCTION)
    {
        let Some(function) = function.as_object() else {
            return Err(RewriteError::Field {
                index,
                field: FUNCTION,
            });
        };
        return Ok(absent_if_null(function.get("name")));
    }
    Ok(absent_if_null(fields.get("name")))
}

// Mirrors is_web_search_tool in LiteLLM's tools.py, over the wire shape so a client schema passes through
pub fn is_web_search_tool(tool: &Json, index: usize) -> Result<bool, RewriteError> {
    let Some(fields) = tool.as_object() else {
        return Err(RewriteError::Tool { index });
    };
    let tool_type = text_field(fields, "type", index)?;
    let tool_name = text_field(fields, "name", index)?;
    if tool_type == Some(FUNCTION)
        && let Some(function) = fields.get(FUNCTION)
    {
        let Some(function) = function.as_object() else {
            return Err(RewriteError::Field {
                index,
                field: FUNCTION,
            });
        };
        if text_field(function, "name", index)? == Some(LITELLM_WEB_SEARCH_TOOL_NAME) {
            return Ok(true);
        }
    }
    if tool_name == Some(LITELLM_WEB_SEARCH_TOOL_NAME) {
        return Ok(true);
    }
    if tool_type.is_some_and(|kind| kind.starts_with(WEB_SEARCH_PREFIX)) {
        return Ok(true);
    }
    if tool_name == Some(WEB_SEARCH) && tool_type.is_some_and(|kind| !kind.is_empty()) {
        return Ok(true);
    }
    Ok(tool_name == Some(LEGACY_NAME) && !fields.contains_key("input_schema"))
}

// Mirrors is_web_search_tool_responses: a non-string type is not a Responses web search tool
pub fn is_web_search_tool_responses(tool: &Json, index: usize) -> Result<bool, RewriteError> {
    let Some(fields) = tool.as_object() else {
        return Err(RewriteError::Tool { index });
    };
    let Some(tool_type) = fields.get("type").and_then(Json::as_str) else {
        return Ok(false);
    };
    if tool_type == FUNCTION {
        return Ok(fields.get("name").and_then(Json::as_str) == Some(LITELLM_WEB_SEARCH_TOOL_NAME));
    }
    Ok(tool_type == WEB_SEARCH || tool_type.starts_with(WEB_SEARCH_PREFIX))
}

pub fn any_tool(
    tools: &[Json],
    matches: fn(&Json, usize) -> Result<bool, RewriteError>,
) -> Result<bool, RewriteError> {
    for (index, tool) in tools.iter().enumerate() {
        if matches(tool, index)? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolFormat {
    Anthropic,
    OpenAi,
    Responses,
}

impl ToolFormat {
    fn matches(self, tool: &Json, index: usize) -> Result<bool, RewriteError> {
        match self {
            Self::Anthropic | Self::OpenAi => is_web_search_tool(tool, index),
            Self::Responses => is_web_search_tool_responses(tool, index),
        }
    }

    fn replacement(self) -> Result<Json, RewriteError> {
        match self {
            Self::Anthropic => encode(&get_litellm_web_search_tool()),
            Self::OpenAi => encode(&openai_web_search_tool()),
            Self::Responses => encode(&responses_web_search_tool()),
        }
    }
}

pub fn rewrite_tools(tools: &[Json], format: ToolFormat) -> Result<Vec<Json>, RewriteError> {
    let mut converted = Vec::with_capacity(tools.len());
    for (index, tool) in tools.iter().enumerate() {
        if format.matches(tool, index)? {
            converted.push(format.replacement()?);
        } else {
            converted.push(tool.clone());
        }
    }
    Ok(converted)
}

// Mirrors _sync_forced_tool_choice: only a forced tool naming a converted-away tool is repointed
pub fn sync_forced_tool_choice(
    tool_choice: &Json,
    converted_tools: &[Json],
) -> Result<Json, RewriteError> {
    let Some(fields) = tool_choice.as_object() else {
        return Ok(tool_choice.clone());
    };
    if fields.get("type").and_then(Json::as_str) != Some("tool") {
        return Ok(tool_choice.clone());
    }
    let chosen = absent_if_null(fields.get("name"));
    let mut names = Vec::with_capacity(converted_tools.len());
    for (index, tool) in converted_tools.iter().enumerate() {
        names.push(effective_name(tool, index)?);
    }
    if names.contains(&chosen) {
        return Ok(tool_choice.clone());
    }
    let mut renamed = fields.clone();
    renamed.insert(
        "name".to_owned(),
        Json::Text(LITELLM_WEB_SEARCH_TOOL_NAME.to_owned()),
    );
    Ok(Json::Object(renamed))
}
