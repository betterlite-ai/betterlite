use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::declared::declared_key;
use super::error::RequestError;
use super::text_format::{merge_text, responses_text_format};
use super::tools::{add_web_search_tool, converted_tools};
use crate::api::response_chat_content::tool_choice::flat_tool_choice;
use crate::api::responses_parameters::stream_options::normalize_stream_options;

pub fn map_optional_params(
    optional_params: &IndexMap<String, Value>,
    params: &mut Map<String, Value>,
    auto_summary: bool,
) -> Result<(), RequestError> {
    for (key, value) in optional_params {
        if value.is_null() {
            continue;
        }
        match key.as_str() {
            "max_tokens" | "max_completion_tokens" => {
                params.insert("max_output_tokens".to_owned(), value.clone());
            }
            "tools" => {
                params.insert("tools".to_owned(), Value::Array(converted_tools(value)?));
            }
            "response_format" => {
                if let Some(format) = responses_text_format(value)? {
                    merge_text(params, "format", format)?;
                }
            }
            "verbosity" => merge_text(params, "verbosity", value.clone())?,
            "tool_choice" => {
                params.insert("tool_choice".to_owned(), flat_tool_choice(value));
            }
            "stream_options" => mapped_stream_options(params, value)?,
            "reasoning_effort" => {
                let reasoning = mapped_reasoning(value, auto_summary);
                params.insert("reasoning".to_owned(), reasoning);
            }
            "web_search_options" => add_web_search_tool(params, value)?,
            _ if declared_key(key) => {
                params.insert(key.clone(), value.clone());
            }
            _ => {}
        }
    }
    Ok(())
}

fn mapped_stream_options(
    params: &mut Map<String, Value>,
    value: &Value,
) -> Result<(), RequestError> {
    let requested: JsonValue =
        serde_json::from_value(value.clone()).map_err(|source| RequestError::Json { source })?;
    let Some(normalized) = normalize_stream_options(Some(&requested)) else {
        return Ok(());
    };
    let encoded =
        serde_json::to_value(normalized).map_err(|source| RequestError::Json { source })?;
    params.insert("stream_options".to_owned(), encoded);
    Ok(())
}

fn mapped_reasoning(value: &Value, auto_summary: bool) -> Value {
    // LiteLLM forwards a reasoning dict verbatim, auto-summary or not
    if value.is_object() {
        return value.clone();
    }
    let mut reasoning = Map::new();
    reasoning.insert("effort".to_owned(), value.clone());
    if auto_summary {
        reasoning.insert("summary".to_owned(), Value::String("detailed".to_owned()));
    }
    Value::Object(reasoning)
}

pub fn effective_stream(
    optional_params: &IndexMap<String, Value>,
    litellm_params: &IndexMap<String, Value>,
) -> Result<bool, RequestError> {
    // LiteLLM reads the option's stream before the SDK params, so an explicit false falls back to the SDK flag
    if requested_flag(optional_params.get("stream"))? {
        return Ok(true);
    }
    requested_flag(litellm_params.get("stream"))
}

fn requested_flag(value: Option<&Value>) -> Result<bool, RequestError> {
    match value {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(Value::Number(_) | Value::String(_) | Value::Array(_) | Value::Object(_)) => {
            Err(RequestError::FieldType {
                field: "stream",
                expected: "a boolean",
            })
        }
    }
}
