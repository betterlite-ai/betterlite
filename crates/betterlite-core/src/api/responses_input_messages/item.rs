use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Value;

use crate::api::responses_codex_tools::records::input_item_role;
use crate::api::responses_input_content::content_blocks;
use crate::api::responses_input_messages::Assembly;
use crate::api::responses_input_messages::content_message;
use crate::api::responses_input_messages::error::InputMessagesError;
use crate::api::responses_input_messages::reasoning::reasoning_messages;
use crate::api::responses_input_messages::web_search::hosted_search_message;
use crate::api::responses_tool_results::input_call::input_function_call;
use crate::api::responses_tool_results::tool_output::tool_call_output_messages;

pub(crate) fn item_messages(
    fields: &IndexMap<String, Json>,
    assembly: &Assembly<'_>,
) -> Result<Vec<Value>, InputMessagesError> {
    let kind = match fields.get("type") {
        Some(Json::Text(text)) => Some(text.as_str()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    };
    if kind == Some("web_search_call") {
        return Ok(vec![hosted_search_message(fields)?]);
    }
    if is_tool_call_output(kind) {
        return tool_output_messages(fields, assembly.cache);
    }
    if is_function_call(kind) {
        let message = input_function_call(fields)?;
        return Ok(vec![serde_json::to_value(message)?]);
    }
    if kind == Some("reasoning") {
        return reasoning_messages(fields, assembly);
    }
    ordinary_message(fields)
}

#[must_use]
pub fn is_tool_call_output(kind: Option<&str>) -> bool {
    matches!(
        kind,
        Some(
            "function_call_output"
                | "custom_tool_call_output"
                | "computer_call_output"
                | "tool_result"
        )
    )
}

#[must_use]
pub fn is_function_call(kind: Option<&str>) -> bool {
    match kind {
        Some("function_call" | "custom_tool_call") => true,
        None | Some(_) => false,
    }
}

fn tool_output_messages(
    fields: &IndexMap<String, Json>,
    cache: &DualLocalCache,
) -> Result<Vec<Value>, InputMessagesError> {
    tool_call_output_messages(fields, cache)?
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()
        .map_err(InputMessagesError::from)
}

fn ordinary_message(fields: &IndexMap<String, Json>) -> Result<Vec<Value>, InputMessagesError> {
    let Some(content) = fields
        .get("content")
        .filter(|value| !matches!(value, Json::Null))
    else {
        return Ok(Vec::new());
    };
    let role = input_item_role(fields)?;
    let converted = content_blocks(&serde_json::to_value(content)?)?;
    Ok(vec![content_message(&role, converted)])
}
