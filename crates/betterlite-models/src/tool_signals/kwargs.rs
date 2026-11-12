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

#[must_use]
pub fn is_web_search_tool_call(tool: &IndexMap<String, Json>) -> bool {
    let matches_type = match tool.get("type") {
        Some(Json::Text(text)) => text == "web_search_preview" || text == "web_search",
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            false
        }
    };
    matches_type || tool.contains_key("search_context_size")
}

#[must_use]
pub fn is_file_search_tool_call(tool: &IndexMap<String, Json>) -> bool {
    match tool.get("type") {
        Some(Json::Text(text)) => text == "file_search",
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            false
        }
    }
}

pub fn get_web_search_options(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<WebSearchOptions>, ToolSignalError> {
    if let Some(explicit) = kwargs.get("web_search_options") {
        let Json::Object(fields) = explicit else {
            return Err(ToolSignalError::OptionsNotMapping);
        };
        return Ok(Some(WebSearchOptions {
            fields: fields.clone(),
        }));
    }
    let Some(tools) = tools_from_kwargs(kwargs, "web_search_preview") else {
        return Ok(None);
    };
    for tool in tool_candidates(tools)? {
        let Json::Object(fields) = tool else {
            continue;
        };
        if is_web_search_tool_call(fields) {
            return Ok(Some(WebSearchOptions {
                fields: fields.clone(),
            }));
        }
    }
    Ok(None)
}

pub fn get_file_search_tool_call(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<FileSearchTool>, ToolSignalError> {
    let Some(tools) = tools_from_kwargs(kwargs, "file_search") else {
        return Ok(None);
    };
    for tool in tool_candidates(tools)? {
        let Json::Object(fields) = tool else {
            continue;
        };
        if is_file_search_tool_call(fields) {
            return Ok(Some(FileSearchTool {
                fields: fields.clone(),
            }));
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
