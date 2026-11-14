use betterlite_context::non_default_params::schema_cleanup::{
    remove_additional_properties, remove_strict_from_schema,
};
use betterlite_models::controls::json_value::JsonValue;
use serde_json::{Value, json};

use super::error::HostedVllmChatError;
use crate::openai_base_config::Object;

const CUSTOM: &str = "custom";
const DESCRIPTION: &str = "description";
const FUNCTION: &str = "function";
const INPUT_SCHEMA: &str = "input_schema";
const NAME: &str = "name";
const PARAMETERS: &str = "parameters";
const TOOL_TYPE: &str = "type";

fn tool_name_value(sources: &[&Object], index: usize) -> Result<String, HostedVllmChatError> {
    for source in sources {
        let Some(given) = source.get(NAME) else {
            continue;
        };
        let Value::String(name) = given else {
            if given.is_null() {
                continue;
            }
            return Err(HostedVllmChatError::CustomToolString { field: NAME });
        };
        if name.is_empty() {
            continue;
        }
        return Ok(name.clone());
    }
    Ok(format!("custom_tool_{index}"))
}

fn tool_description_value(sources: &[&Object]) -> Result<Option<String>, HostedVllmChatError> {
    for (index, source) in sources.iter().enumerate() {
        let Some(given) = source.get(DESCRIPTION) else {
            continue;
        };
        let Value::String(text) = given else {
            if given.is_null() {
                continue;
            }
            return Err(HostedVllmChatError::CustomToolString { field: DESCRIPTION });
        };
        // the last source is the fallback's final operand, so its empty string is used as given
        if text.is_empty() && index + 1 < sources.len() {
            continue;
        }
        return Ok(Some(text.clone()));
    }
    Ok(None)
}

fn tool_parameters_value(sources: &[&Object]) -> Result<Value, HostedVllmChatError> {
    for (index, source) in sources.iter().enumerate() {
        let Some(given) = source.get(INPUT_SCHEMA) else {
            continue;
        };
        let Value::Object(schema) = given else {
            if given.is_null() {
                continue;
            }
            return Err(HostedVllmChatError::CustomToolSchema);
        };
        // the last source is the fallback's final operand, so its empty object is used as given
        if schema.is_empty() && index + 1 < sources.len() {
            continue;
        }
        return Ok(given.clone());
    }
    Ok(json!({
        "type": "object",
        "properties": {"input": {"type": "string", "description": "Raw tool input payload."}},
        "required": ["input"],
    }))
}

fn convert_custom_tool(fields: &Object, index: usize) -> Result<Value, HostedVllmChatError> {
    let custom = match fields.get(CUSTOM) {
        None | Some(Value::Null) => None,
        Some(Value::Object(custom)) => Some(custom),
        Some(_) => return Err(HostedVllmChatError::CustomToolObject),
    };
    let mut sources: Vec<&Object> = Vec::with_capacity(2);
    if let Some(custom) = custom {
        sources.push(custom);
    }
    sources.push(fields);
    let name = tool_name_value(&sources, index)?;
    let description = tool_description_value(&sources)?;
    let parameters = tool_parameters_value(&sources)?;
    let mut function = Object::new();
    function.insert(String::from(NAME), Value::String(name));
    function.insert(String::from(PARAMETERS), parameters);
    if let Some(description) = description {
        function.insert(String::from(DESCRIPTION), Value::String(description));
    }
    Ok(Value::Object(Object::from_iter([
        (
            String::from(TOOL_TYPE),
            Value::String(String::from(FUNCTION)),
        ),
        (String::from(FUNCTION), Value::Object(function)),
    ])))
}

fn convert_tools(items: Vec<Value>) -> Result<Vec<Value>, HostedVllmChatError> {
    let mut converted = Vec::with_capacity(items.len());
    for (index, tool) in items.into_iter().enumerate() {
        let Value::Object(fields) = tool else {
            return Err(HostedVllmChatError::ToolNotObject { index });
        };
        if fields.get(TOOL_TYPE).and_then(Value::as_str) != Some(CUSTOM) {
            converted.push(Value::Object(fields));
            continue;
        }
        converted.push(convert_custom_tool(&fields, index)?);
    }
    Ok(converted)
}

pub(crate) fn clean_and_convert(tools: Value) -> Result<Value, HostedVllmChatError> {
    let mut schema = serde_json::from_value::<JsonValue>(tools)
        .map_err(|source| HostedVllmChatError::Json { source })?;
    remove_additional_properties(&mut schema);
    remove_strict_from_schema(&mut schema);
    let cleaned =
        serde_json::to_value(schema).map_err(|source| HostedVllmChatError::Json { source })?;
    match cleaned {
        Value::Array(items) => Ok(Value::Array(convert_tools(items)?)),
        other => Ok(other),
    }
}
