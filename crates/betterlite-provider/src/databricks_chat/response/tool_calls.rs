use betterlite_prompts::arguments::json::{Json, Object};

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::function::{NormalizedFunction, normalize_function};
use crate::databricks_chat::response::reply::{ReplyFunction, ReplyToolCall};
use crate::databricks_chat::wire::render;
use crate::ollama_chat::ids::IdSource;

const CUSTOM: &str = "custom";
const FUNCTION: &str = "function";
const JSON_TOOL_CALL: &str = "json_tool_call";
const MULTI_TOOL: &str = "multi_tool_use.parallel";
const PARAMETERS: &str = "parameters";
const RECIPIENT: &str = "recipient_name";
const TOOL_USES: &str = "tool_uses";

// Message(tool_calls=...) keeps a non-empty list and turns anything else into null
pub(super) fn normalize(
    tool_calls: Option<&Json>,
    ids: &dyn IdSource,
) -> Result<Option<Vec<ReplyToolCall>>, DatabricksChatError> {
    let Some(value) = tool_calls else {
        return Ok(None);
    };
    let items = match value {
        Json::Null => return Ok(None),
        Json::List(items) => items,
        Json::Text(text) if text.is_empty() => return Ok(None),
        Json::Object(fields) if fields.is_empty() => return Ok(None),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            return Err(DatabricksChatError::wrong_type("tool_calls", "a list"));
        }
    };
    if items.is_empty() {
        return Ok(None);
    }
    let mut calls = Vec::with_capacity(items.len());
    for item in items {
        calls.push(normalize_call(item, ids)?);
    }
    Ok(Some(calls))
}

fn normalize_call(item: &Json, ids: &dyn IdSource) -> Result<ReplyToolCall, DatabricksChatError> {
    let Some(fields) = item.as_object() else {
        return Err(DatabricksChatError::wrong_type("tool call", "an object"));
    };
    let is_custom = fields.get("type").and_then(Json::as_str) == Some(CUSTOM)
        || !matches!(fields.get(CUSTOM), None | Some(Json::Null));
    if is_custom {
        return Err(DatabricksChatError::Shape {
            message: String::from("a custom tool call in a Databricks reply is not supported"),
        });
    }
    let function = match fields.get(FUNCTION) {
        Some(Json::Object(function)) => function,
        None | Some(Json::Null) => return Err(DatabricksChatError::missing("tool call", FUNCTION)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(DatabricksChatError::wrong_type(
                "function of a tool call",
                "an object",
            ));
        }
    };
    let id = match fields.get("id") {
        None | Some(Json::Null) => ids.next_id(),
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(DatabricksChatError::wrong_type(
                "id of a tool call",
                "a string",
            ));
        }
    };
    let kind = match fields.get("type") {
        None | Some(Json::Null) => String::from("function"),
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(DatabricksChatError::wrong_type(
                "type of a tool call",
                "a string",
            ));
        }
    };
    let mut extra = fields.clone();
    for named in [FUNCTION, "id", "type"] {
        extra.shift_remove(named);
    }
    let NormalizedFunction { arguments, name } = normalize_function(function)?;
    Ok(ReplyToolCall {
        extra,
        function: ReplyFunction { arguments, name },
        id,
        kind,
    })
}

// _handle_invalid_parallel_tool_calls expands a multi_tool_use.parallel call into its uses
pub(super) fn repair(calls: &[ReplyToolCall]) -> Result<Vec<ReplyToolCall>, DatabricksChatError> {
    let mut repaired = Vec::with_capacity(calls.len());
    for call in calls {
        if call.function.name.as_deref() != Some(MULTI_TOOL) {
            repaired.push(call.clone());
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Json>(&call.function.arguments) else {
            return Ok(calls.to_vec());
        };
        let Some(arguments) = parsed.as_object() else {
            return Err(DatabricksChatError::wrong_type(
                "arguments of a multi_tool_use.parallel call",
                "an object",
            ));
        };
        let uses = match arguments.get(TOOL_USES) {
            Some(Json::List(uses)) => uses,
            None => {
                return Err(DatabricksChatError::Shape {
                    message: String::from(
                        "a multi_tool_use.parallel call is missing its tool_uses",
                    ),
                });
            }
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
            ) => {
                return Err(DatabricksChatError::wrong_type(
                    "tool_uses of a multi_tool_use.parallel call",
                    "a list",
                ));
            }
        };
        for (position, tool_use) in uses.iter().enumerate() {
            repaired.push(expand(call, tool_use, position)?);
        }
    }
    Ok(repaired)
}

fn expand(
    call: &ReplyToolCall,
    tool_use: &Json,
    position: usize,
) -> Result<ReplyToolCall, DatabricksChatError> {
    let Some(fields) = tool_use.as_object() else {
        return Err(DatabricksChatError::wrong_type(
            "tool_use of a multi_tool_use.parallel call",
            "an object",
        ));
    };
    let recipient = match fields.get(RECIPIENT) {
        Some(Json::Text(recipient)) => recipient,
        None => {
            return Err(DatabricksChatError::missing(
                "multi_tool_use.parallel tool_use",
                RECIPIENT,
            ));
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(DatabricksChatError::wrong_type(
                "recipient_name of a multi_tool_use.parallel tool_use",
                "a string",
            ));
        }
    };
    let Some(parameters) = fields.get(PARAMETERS) else {
        return Err(DatabricksChatError::missing(
            "multi_tool_use.parallel tool_use",
            PARAMETERS,
        ));
    };
    let name = recipient.strip_prefix("functions.").unwrap_or(recipient);
    Ok(ReplyToolCall {
        extra: Object::new(),
        function: ReplyFunction {
            arguments: render(parameters),
            name: Some(name.to_owned()),
        },
        id: format!("{}_{position}", call.id),
        kind: String::from("function"),
    })
}

// _should_convert_tool_call_to_json_mode and the content it builds from the arguments
pub(super) fn json_mode_content(calls: &[ReplyToolCall], json_mode: bool) -> Option<String> {
    if !json_mode || calls.len() != 1 {
        return None;
    }
    let call = &calls[0];
    if call.function.name.as_deref() != Some(JSON_TOOL_CALL) {
        return None;
    }
    if call.function.arguments.is_empty() {
        return None;
    }
    Some(call.function.arguments.clone())
}
