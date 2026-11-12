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
    if let Some(explicit) = kwargs.get("web_search_options") {
        let Json::Object(fields) = explicit else {
            return Err(ToolSignalError::OptionsNotMapping);
        };
        return Ok(Some(WebSearchOptions {
            fields: fields.clone(),
        }));
    }
    // The requested tool type is ignored: the whole tools list is scanned and the first match wins.
    let found = first_matching_tool(kwargs, is_web_search_tool_call)?;
    Ok(found.map(|fields| WebSearchOptions { fields }))
}

pub fn get_file_search_tool_call(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<FileSearchTool>, ToolSignalError> {
    // File search scans the same unfiltered list independently of the requested tool type.
    let Some(tools) = kwargs.get("tools") else {
        return Ok(None);
    };
    for tool in tool_candidates(tools)? {
        if let Json::Object(fields) = tool
            && tool_type(fields) == Some("file_search")
        {
            return Ok(Some(FileSearchTool {
                fields: fields.clone(),
            }));
        }
    }
    Ok(None)
}

fn first_matching_tool(
    kwargs: &IndexMap<String, Json>,
    matches: fn(&IndexMap<String, Json>) -> bool,
) -> Result<Option<IndexMap<String, Json>>, ToolSignalError> {
    let Some(tools) = kwargs.get("tools") else {
        return Ok(None);
    };
    for tool in tool_candidates(tools)? {
        if let Json::Object(fields) = tool
            && matches(fields)
        {
            return Ok(Some(fields.clone()));
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
