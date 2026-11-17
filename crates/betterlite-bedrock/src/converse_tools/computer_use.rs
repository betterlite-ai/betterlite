use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::converse_tools::error::ConverseToolsError;

pub const BEDROCK_COMPUTER_USE_TOOLS: [&str; 4] =
    ["computer_use_preview", "computer_", "bash_", "text_editor_"];

pub fn is_computer_use_tool_used(tools: Option<&[Json]>) -> Result<bool, ConverseToolsError> {
    let Some(tools) = tools else {
        return Ok(false);
    };
    for tool in tools {
        if tool_type(tool)?.is_some_and(is_computer_use_type) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn separate_computer_use_tools(
    tools: &[Json],
) -> Result<(Vec<Json>, Vec<Json>), ConverseToolsError> {
    let mut computer = Vec::new();
    let mut regular = Vec::new();
    for tool in tools {
        if tool_type(tool)?.is_some_and(is_computer_use_type) {
            computer.push(tool.clone());
        } else {
            regular.push(tool.clone());
        }
    }
    Ok((computer, regular))
}

pub fn transform_computer_use_tools(tools: &[Json]) -> Result<Vec<Json>, ConverseToolsError> {
    let mut transformed = Vec::with_capacity(tools.len());
    for tool in tools {
        transformed.push(transform_tool(tool)?);
    }
    Ok(transformed)
}

fn is_computer_use_type(tool_type: &str) -> bool {
    BEDROCK_COMPUTER_USE_TOOLS
        .into_iter()
        .any(|prefix| tool_type.starts_with(prefix))
}

fn tool_type(tool: &Json) -> Result<Option<&str>, ConverseToolsError> {
    let Json::Object(fields) = tool else {
        return Err(ConverseToolsError::ToolNotAnObject);
    };
    match fields.get("type") {
        None => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ConverseToolsError::ToolType)
        }
    }
}

// The transformer reads a missing type as "not a computer tool" and copies the tool unchanged.
fn transform_tool(tool: &Json) -> Result<Json, ConverseToolsError> {
    let Json::Object(fields) = tool else {
        return Err(ConverseToolsError::ToolNotAnObject);
    };
    match fields.get("type") {
        None => Ok(tool.clone()),
        Some(Json::Text(tool_type)) => typed_tool(fields, tool_type),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ConverseToolsError::ToolType)
        }
    }
}

fn typed_tool(
    fields: &IndexMap<String, Json>,
    tool_type: &str,
) -> Result<Json, ConverseToolsError> {
    if !is_computer_use_type(tool_type) {
        return Ok(Json::Object(fields.clone()));
    }
    if tool_type.starts_with("computer_")
        && let Some(function) = fields.get("function")
    {
        return function_tool(tool_type, function);
    }
    Ok(direct_tool(fields, tool_type))
}

// The function parameters spread last in LiteLLM, so they can replace the generated type and name.
fn function_tool(tool_type: &str, function: &Json) -> Result<Json, ConverseToolsError> {
    let Json::Object(function) = function else {
        return Err(ConverseToolsError::ToolFunction);
    };
    let mut fields = IndexMap::new();
    fields.insert("type".to_owned(), Json::Text(tool_type.to_owned()));
    let name = function
        .get("name")
        .map_or_else(|| Json::Text("computer".to_owned()), Clone::clone);
    fields.insert("name".to_owned(), name);
    match function.get("parameters") {
        None => {}
        Some(Json::Object(parameters)) => {
            for (key, value) in parameters {
                fields.insert(key.clone(), value.clone());
            }
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(ConverseToolsError::ToolParameters);
        }
    }
    Ok(Json::Object(fields))
}

fn direct_tool(fields: &IndexMap<String, Json>, tool_type: &str) -> Json {
    let mut transformed = fields.clone();
    if !transformed.contains_key("name") {
        let name = if tool_type.starts_with("bash_") {
            Some("bash")
        } else if tool_type.starts_with("text_editor_") {
            Some("str_replace_editor")
        } else {
            None
        };
        if let Some(name) = name {
            transformed.insert("name".to_owned(), Json::Text(name.to_owned()));
        }
    }
    Json::Object(transformed)
}
