use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::arguments::shapes::{nested_custom_grammar, nested_to_flat_grammar};
use serde_json::{Map, Value};

use super::error::RequestError;

pub fn converted_tools(tools: &Value) -> Result<Vec<Value>, RequestError> {
    let Value::Array(items) = tools else {
        return Err(RequestError::FieldType {
            field: "tools",
            expected: "a list of objects",
        });
    };
    items.iter().map(converted_tool).collect()
}

// a tool LiteLLM doesn't recognize keeps its chat shape, which the Responses API passes through
fn converted_tool(tool: &Value) -> Result<Value, RequestError> {
    let Value::Object(fields) = tool else {
        return Err(RequestError::FieldType {
            field: "tools",
            expected: "a list of objects",
        });
    };
    match fields.get("type").and_then(Value::as_str) {
        Some("function") => match fields.get("function") {
            Some(Value::Object(function)) => function_tool(function),
            _ => Ok(tool.clone()),
        },
        Some("custom") => match fields.get("custom") {
            Some(Value::Object(custom)) => custom_tool(custom),
            _ => Ok(tool.clone()),
        },
        _ => Ok(tool.clone()),
    }
}

fn function_tool(function: &Map<String, Value>) -> Result<Value, RequestError> {
    // LiteLLM reads the name without a default and raises, so a nameless function tool is the caller's error
    let Some(name) = function.get("name").and_then(Value::as_str) else {
        return Err(RequestError::FieldType {
            field: "tools",
            expected: "function tools with a string name",
        });
    };
    let mut converted = Map::new();
    converted.insert("name".to_owned(), Value::String(name.to_owned()));
    converted.insert("type".to_owned(), Value::String("function".to_owned()));
    // the Responses function tool always carries the three chat fields, absent ones as null
    for (key, value) in [
        ("parameters", function.get("parameters")),
        ("strict", function.get("strict")),
        ("description", function.get("description")),
    ] {
        converted.insert(key.to_owned(), value.cloned().unwrap_or(Value::Null));
    }
    Ok(Value::Object(converted))
}

fn custom_tool(custom: &Map<String, Value>) -> Result<Value, RequestError> {
    let mut converted = Map::new();
    converted.insert("type".to_owned(), Value::String("custom".to_owned()));
    let name = match custom.get("name") {
        None => String::new(),
        Some(Value::String(name)) => name.clone(),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_),
        ) => {
            return Err(RequestError::FieldType {
                field: "tools",
                expected: "custom tools with a string name",
            });
        }
    };
    converted.insert("name".to_owned(), Value::String(name));
    if let Some(description) = custom.get("description").filter(|value| !value.is_null()) {
        converted.insert("description".to_owned(), description.clone());
    }
    if let Some(Value::Object(format)) = custom.get("format") {
        converted.insert("format".to_owned(), flat_custom_format(format)?);
    }
    Ok(Value::Object(converted))
}

// B216's inverse of the chat grammar shape, applied only to the nested grammar form
fn flat_custom_format(format: &Map<String, Value>) -> Result<Value, RequestError> {
    let nested: Object = serde_json::from_value(Value::Object(format.clone()))
        .map_err(|source| RequestError::Json { source })?;
    let Some(grammar) = nested_custom_grammar(&nested) else {
        return Ok(Value::Object(format.clone()));
    };
    serde_json::to_value(nested_to_flat_grammar(grammar))
        .map_err(|source| RequestError::Json { source })
}

pub fn add_web_search_tool(
    params: &mut Map<String, Value>,
    options: &Value,
) -> Result<(), RequestError> {
    if params.get("tools").is_none_or(Value::is_null) {
        params.insert("tools".to_owned(), Value::Array(Vec::new()));
    }
    let Some(Value::Array(tools)) = params.get_mut("tools") else {
        return Err(RequestError::FieldType {
            field: "tools",
            expected: "a list of objects",
        });
    };
    let mut tool = Map::new();
    tool.insert("type".to_owned(), Value::String("web_search".to_owned()));
    if let Value::Object(fields) = options {
        for (key, value) in fields {
            tool.insert(key.clone(), value.clone());
        }
    }
    tools.push(Value::Object(tool));
    Ok(())
}
