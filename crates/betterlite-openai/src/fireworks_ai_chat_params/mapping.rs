use std::num::FpCategory;

use betterlite_context::provider_message_primitives::{
    ChoicePolicy, FormatParams, FormatSupport, NestedSchema, ResponseFormat, Schema, ToolParam,
    add_response_format,
};
use betterlite_models::fireworks_catalog_metadata::Catalog;
use serde_json::Value;

use super::FireworksCapabilities;
use super::FireworksChatParamsError;
use super::supported::supported_openai_params;
use crate::openai_base_config::Object;

// LiteLLM's map_openai_params: the supported list, then every special branch, in the caller's key order.
pub async fn map_openai_params<P>(
    capabilities: &mut P,
    catalog: &Catalog,
    model: &str,
    non_default_params: &[(String, Value)],
    optional_params: &mut Object,
) -> Result<(), FireworksChatParamsError>
where
    P: FireworksCapabilities,
{
    let supported = supported_openai_params(capabilities, catalog, model).await;
    let is_tools_set = non_default_params
        .iter()
        .any(|(name, value)| name == "tools" && !value.is_null());
    let thinking = last_value_set(non_default_params, "thinking");
    let effort = last_value_set(non_default_params, "reasoning_effort");
    if thinking && effort {
        return Err(FireworksChatParamsError::ThinkingConflict {
            model: model.to_owned(),
        });
    }
    for (name, value) in non_default_params {
        map_parameter(name, value, &supported, is_tools_set, optional_params)?;
    }
    Ok(())
}

fn map_parameter(
    name: &str,
    value: &Value,
    supported: &[&'static str],
    is_tools_set: bool,
    optional_params: &mut Object,
) -> Result<(), FireworksChatParamsError> {
    match name {
        "tool_choice" => {
            let mapped = if value.as_str() == Some("required") {
                Value::String(String::from("any"))
            } else {
                value.clone()
            };
            optional_params.insert(name.to_owned(), mapped);
        }
        "response_format" => apply_response_format(optional_params, value, is_tools_set)?,
        "max_completion_tokens" => {
            optional_params.insert(String::from("max_tokens"), value.clone());
        }
        "reasoning_effort" => {
            if let Some(mapped) = map_reasoning_effort(value) {
                optional_params.insert(name.to_owned(), mapped);
            }
        }
        _ if supported.contains(&name) && !value.is_null() => {
            optional_params.insert(name.to_owned(), value.clone());
        }
        _ => {}
    }
    Ok(())
}

fn last_value_set(non_default_params: &[(String, Value)], name: &str) -> bool {
    non_default_params
        .iter()
        .rev()
        .find(|(key, _)| key == name)
        .is_some_and(|(_, value)| !value.is_null())
}

// LiteLLM's _map_reasoning_effort: a mapping contributes its effort entry, and none, false and auto each map away.
fn map_reasoning_effort(value: &Value) -> Option<Value> {
    let chosen = match value {
        Value::Object(fields) => fields.get("effort"),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Some(value)
        }
    };
    let effort = chosen?;
    match effort {
        Value::Null => None,
        Value::Bool(true) => Some(Value::String(String::from("medium"))),
        Value::Bool(false) => Some(Value::String(String::from("none"))),
        Value::String(text) if text == "auto" => None,
        Value::String(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            Some(effort.clone())
        }
    }
}

fn apply_response_format(
    optional_params: &mut Object,
    value: &Value,
    is_tools_set: bool,
) -> Result<(), FireworksChatParamsError> {
    if !is_tools_set {
        optional_params.insert(String::from("response_format"), value.clone());
        return Ok(());
    }
    let Value::Object(format) = value else {
        return Err(FireworksChatParamsError::ResponseFormatNotObject);
    };
    let malformed_tools = optional_params
        .get("tools")
        .is_some_and(|tools| !matches!(tools, Value::Array(_)));
    let mut params = format_params(optional_params);
    add_response_format(
        &mut params,
        response_format_record(format),
        FormatSupport::ToolCall,
        ChoicePolicy::KeepExisting,
    )
    .map_err(|source| FireworksChatParamsError::ResponseFormat { source })?;
    if malformed_tools && params.json_mode == Some(true) {
        return Err(FireworksChatParamsError::ToolsNotList);
    }
    merge_format_params(optional_params, &params)
}

fn format_params(optional_params: &Object) -> FormatParams<Value> {
    let tools = match optional_params.get("tools") {
        Some(Value::Array(items)) => Some(items.iter().cloned().map(ToolParam::Supplied).collect()),
        Some(_) | None => None,
    };
    FormatParams {
        tools,
        tool_choice: None,
        json_mode: None,
        response_format: None,
    }
}

fn merge_format_params(
    optional_params: &mut Object,
    params: &FormatParams<Value>,
) -> Result<(), FireworksChatParamsError> {
    let encoded = serde_json::to_value(params)
        .map_err(|source| FireworksChatParamsError::Encode { source })?;
    let produced: Value = serde_json::from_value(encoded)
        .map_err(|source| FireworksChatParamsError::Encode { source })?;
    if let Value::Object(fields) = produced {
        optional_params.extend(fields);
    }
    Ok(())
}

fn response_format_record(format: &Object) -> ResponseFormat<Value> {
    ResponseFormat {
        value: Value::Object(format.clone()),
        response_schema: format.get("response_schema").map(schema_record),
        json_schema: format.get("json_schema").map(|nested| match nested {
            Value::Object(fields) => NestedSchema::Object(fields.get("schema").map(schema_record)),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_) => NestedSchema::NotObject,
        }),
    }
}

fn schema_record(value: &Value) -> Schema<Value> {
    match value {
        Value::Null | Value::Bool(false) => Schema::Falsy,
        Value::Array(items) if items.is_empty() => Schema::Falsy,
        Value::Object(fields) if fields.is_empty() => Schema::Falsy,
        Value::String(text) if text.is_empty() => Schema::Falsy,
        Value::Number(number)
            if number
                .as_f64()
                .is_some_and(|parsed| parsed.classify() == FpCategory::Zero) =>
        {
            Schema::Falsy
        }
        Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_)
        | Value::Object(_) => Schema::Truthy(value.clone()),
    }
}
