use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::decisions::Json;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tool_signals::error::ToolSignalError;

// The pinned WebSearchOptions and FileSearchTool TypedDicts copy the whole mapping, extras and bad values included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchOptions {
    pub fields: IndexMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSearchTool {
    pub fields: IndexMap<String, Json>,
}

// The requested tool_type is ignored: a caller's tools list is returned whole and each scan decides the kind.
#[must_use]
pub fn tools_from_kwargs<'kwargs>(
    kwargs: &'kwargs IndexMap<String, Json>,
    _tool_type: &str,
) -> Option<&'kwargs Json> {
    if kwargs.contains_key("tools") {
        return kwargs.get("tools");
    }
    None
}

fn tool_type(tool: &IndexMap<String, Json>) -> Option<&str> {
    match tool.get("type") {
        Some(Json::Text(text)) => Some(text),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

#[must_use]
pub fn is_web_search_tool_call(tool: &IndexMap<String, Json>) -> bool {
    matches!(tool_type(tool), Some("web_search_preview" | "web_search"))
        || tool.contains_key("search_context_size")
}

pub fn get_web_search_options(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<WebSearchOptions>, ToolSignalError> {
    let explicit = match kwargs.get("web_search_options") {
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => return Err(ToolSignalError::OptionsNotMapping),
        None => first_matching_tool(kwargs, "web_search_preview", is_web_search_tool_call)?,
    };
    Ok(explicit.map(|fields| WebSearchOptions {
        fields: fields.clone(),
    }))
}

pub fn get_file_search_tool_call(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<FileSearchTool>, ToolSignalError> {
    Ok(first_matching_tool(kwargs, "file_search", |tool| {
        tool_type(tool) == Some("file_search")
    })?
    .cloned()
    .map(|fields| FileSearchTool { fields }))
}

fn first_matching_tool<'kwargs>(
    kwargs: &'kwargs IndexMap<String, Json>,
    tool_type: &str,
    matches: fn(&IndexMap<String, Json>) -> bool,
) -> Result<Option<&'kwargs IndexMap<String, Json>>, ToolSignalError> {
    let Some(tools) = tools_from_kwargs(kwargs, tool_type) else {
        return Ok(None);
    };
    for tool in tool_candidates(tools)? {
        if let Json::Object(fields) = tool
            && matches(fields)
        {
            return Ok(Some(fields));
        }
    }
    Ok(None)
}

// A tools value is iterated only when it is truthy and iterable; empty values and every non-dict member yield none.
fn tool_candidates(tools: &Json) -> Result<&[Json], ToolSignalError> {
    match tools {
        Json::List(items) => Ok(items),
        Json::Null | Json::Bool(false) | Json::Text(_) | Json::Object(_) => Ok(&[]),
        Json::Bool(true) => Err(ToolSignalError::ToolsNotIterable),
        Json::Number(number) => {
            if parse_decimal_lexeme(&number.to_string()).is_ok_and(|value| value.is_zero()) {
                Ok(&[])
            } else {
                Err(ToolSignalError::ToolsNotIterable)
            }
        }
    }
}

mod tests;
