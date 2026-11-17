use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::converse_tools::error::ConverseToolsError;

pub const BEDROCK_COMPUTER_USE_TOOLS: [&str; 4] =
    ["computer_use_preview", "computer_", "bash_", "text_editor_"];

pub fn is_computer_use_tool_used(tools: Option<&[Json]>) -> Result<bool, ConverseToolsError> {
    let Some(list) = tools else {
        return Ok(false);
    };
    for tool in list {
        if tool_type(fields_of(tool)?)?.is_some_and(is_computer_use_type) {
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
        if tool_type(fields_of(tool)?)?.is_some_and(is_computer_use_type) {
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
        // A missing type reads as not a computer tool, so the tool is copied unchanged.
        let fields = fields_of(tool)?;
        transformed.push(
            tool_type(fields)?.map_or_else(|| Ok(tool.clone()), |kind| typed_tool(fields, kind))?,
        );
    }
    Ok(transformed)
}

fn is_computer_use_type(tool_type: &str) -> bool {
    BEDROCK_COMPUTER_USE_TOOLS
        .into_iter()
        .any(|prefix| tool_type.starts_with(prefix))
}

const fn fields_of(tool: &Json) -> Result<&IndexMap<String, Json>, ConverseToolsError> {
    let Json::Object(fields) = tool else {
        return Err(ConverseToolsError::ToolNotAnObject);
    };
    Ok(fields)
}

fn tool_type(fields: &IndexMap<String, Json>) -> Result<Option<&str>, ConverseToolsError> {
    match fields.get("type") {
        None => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
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
    let Json::Object(members) = function else {
        return Err(ConverseToolsError::ToolFunction);
    };
    let mut fields = IndexMap::new();
    fields.insert("type".to_owned(), Json::Text(tool_type.to_owned()));
    let name = members
        .get("name")
        .map_or_else(|| Json::Text("computer".to_owned()), Clone::clone);
    fields.insert("name".to_owned(), name);
    match members.get("parameters") {
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
        let fallback = if tool_type.starts_with("bash_") {
            Some("bash")
        } else if tool_type.starts_with("text_editor_") {
            Some("str_replace_editor")
        } else {
            None
        };
        if let Some(name) = fallback {
            transformed.insert("name".to_owned(), Json::Text(name.to_owned()));
        }
    }
    Json::Object(transformed)
}
