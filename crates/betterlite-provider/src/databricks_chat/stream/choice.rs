use betterlite_openai::chat::choice_finish_reason;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_json::Number;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::stream::ChunkParser;
use crate::databricks_chat::stream::citation;
use crate::databricks_chat::stream::delta::{Delta, build_delta};
use crate::databricks_chat::wire::render;

const JSON_TOOL_CALL: &str = "json_tool_call";

// StreamingChoices: finish reasons map, index and logprobs pass through, extras stay
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamChoice {
    pub finish_reason: Option<String>,
    pub index: Json,
    pub delta: Delta,
    pub logprobs: Json,
    #[serde(flatten)]
    pub extra: Object,
}

pub(super) fn transform(
    choice: &Json,
    parser: &mut ChunkParser,
    rendered: &str,
) -> Result<StreamChoice, DatabricksChatError> {
    let Some(fields) = choice.as_object() else {
        return Err(DatabricksChatError::shape(
            "a choice of a Databricks stream chunk must be an object",
        ));
    };
    let mut delta = match fields.get("delta") {
        Some(Json::Object(delta)) => delta.clone(),
        None => return Err(DatabricksChatError::chunk_key("delta", rendered)),
        Some(_) => {
            return Err(DatabricksChatError::wrong_type(
                "delta of a choice",
                "an object",
            ));
        }
    };
    prepare_delta(&mut delta, parser, rendered)?;
    Ok(StreamChoice {
        finish_reason: finish_reason_of(fields)?,
        index: match fields.get("index") {
            None => Json::Number(Number::from(0)),
            Some(index) => index.clone(),
        },
        delta: build_delta(&delta)?,
        logprobs: fields.get("logprobs").cloned().unwrap_or(Json::Null),
        extra: extra_of(fields),
    })
}

// An empty finish reason is absent; a truthy one maps like any OpenAI-compatible finish reason
fn finish_reason_of(fields: &Object) -> Result<Option<String>, DatabricksChatError> {
    let Some(value) = fields.get("finish_reason") else {
        return Ok(None);
    };
    match value {
        Json::Null | Json::Bool(false) => Ok(None),
        Json::Text(text) if text.is_empty() => Ok(None),
        Json::Text(text) => Ok(Some(choice_finish_reason(Some(text)).0)),
        Json::Bool(true) => Ok(Some(String::from("stop"))),
        Json::Number(number) => Ok(number
            .as_f64()
            .is_some_and(|parsed| parsed != 0.0_f64)
            .then(|| String::from("stop"))),
        Json::List(items) if items.is_empty() => Ok(None),
        Json::Object(entries) if entries.is_empty() => Ok(None),
        Json::List(_) | Json::Object(_) => Err(DatabricksChatError::wrong_type(
            "finish_reason of a Databricks stream chunk",
            "a string",
        )),
    }
}

fn extra_of(fields: &Object) -> Object {
    let mut extra = fields.clone();
    for name in ["finish_reason", "index", "delta", "logprobs", "message"] {
        extra.shift_remove(name);
    }
    extra
}

// The json-mode conversion, the empty-arguments cleanup and the citation all mutate the delta
fn prepare_delta(
    delta: &mut Object,
    parser: &mut ChunkParser,
    rendered: &str,
) -> Result<(), DatabricksChatError> {
    let tool_calls = delta.get("tool_calls").cloned();
    if let Some(tool_calls) = tool_calls
        && tool_calls.is_truthy()
    {
        if parser.json_mode {
            convert_tool_calls(delta, &tool_calls, parser, rendered)?;
        } else {
            let mut tool_calls = tool_calls;
            clear_empty_arguments(&mut tool_calls)?;
            delta.insert(String::from("tool_calls"), tool_calls);
        }
    }
    citation::add(delta)
}

// _convert_tool_response_to_message: a json_tool_call's arguments become the delta content
fn convert_tool_calls(
    delta: &mut Object,
    tool_calls: &Json,
    parser: &mut ChunkParser,
    rendered: &str,
) -> Result<(), DatabricksChatError> {
    let Some((function, function_name)) = first_call(tool_calls)? else {
        return Ok(());
    };
    if let Some(name) = &function_name {
        parser.last_function_name = Some(name.clone());
    }
    let matched = parser.last_function_name.as_deref() == Some(JSON_TOOL_CALL)
        || function_name.as_deref() == Some(JSON_TOOL_CALL);
    if !matched {
        return Ok(());
    }
    let Some(function) = function else {
        return Err(DatabricksChatError::chunk_key("function", rendered));
    };
    write_json_mode_content(delta, function)
}

// The first tool call's function and its name, each only when the chunk carries it
fn first_call(
    tool_calls: &Json,
) -> Result<Option<(Option<&Object>, Option<String>)>, DatabricksChatError> {
    let Json::List(calls) = tool_calls else {
        return Err(DatabricksChatError::wrong_type(
            "tool_calls of a Databricks stream delta",
            "a list",
        ));
    };
    let Some(first) = calls.first() else {
        return Ok(None);
    };
    let Some(first) = first.as_object() else {
        return Err(DatabricksChatError::wrong_type(
            "a tool call of a Databricks stream delta",
            "an object",
        ));
    };
    let function = match first.get("function") {
        None => None,
        Some(Json::Object(function)) => Some(function),
        Some(_) => {
            return Err(DatabricksChatError::wrong_type(
                "function of a tool call",
                "an object",
            ));
        }
    };
    let name = match function {
        None => None,
        Some(function) => match function.get("name") {
            None | Some(Json::Null) => None,
            Some(Json::Text(text)) => Some(text.clone()),
            Some(_) => {
                return Err(DatabricksChatError::wrong_type(
                    "name of a tool call function",
                    "a string",
                ));
            }
        },
    };
    Ok(Some((function, name)))
}

fn write_json_mode_content(
    delta: &mut Object,
    function: &Object,
) -> Result<(), DatabricksChatError> {
    let Some(arguments) = function.get("arguments") else {
        return Ok(());
    };
    let Json::Text(arguments) = arguments else {
        if matches!(arguments, Json::Null) {
            return Ok(());
        }
        return Err(DatabricksChatError::wrong_type(
            "arguments of a json mode tool call",
            "a string",
        ));
    };
    delta.insert(
        String::from("content"),
        Json::Text(convert_arguments(arguments)),
    );
    delta.insert(String::from("tool_calls"), Json::Null);
    Ok(())
}

// json.loads, then the values key when it holds something, and "{}" becomes ""
fn convert_arguments(raw: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<Json>(raw) else {
        return raw.to_owned();
    };
    let content = match &parsed {
        Json::Object(fields) => match fields.get("values") {
            Some(Json::Null) | None => render(&parsed),
            Some(values) => render(values),
        },
        _ => render(&parsed),
    };
    if content == "{}" {
        String::new()
    } else {
        content
    }
}

// The non-json path rewrites a "{}" arguments value so the caller never sees invalid json
fn clear_empty_arguments(tool_calls: &mut Json) -> Result<(), DatabricksChatError> {
    let Json::List(calls) = tool_calls else {
        return Err(DatabricksChatError::wrong_type(
            "tool_calls of a Databricks stream delta",
            "a list",
        ));
    };
    for call in calls {
        let Some(fields) = call.as_object_mut() else {
            return Err(DatabricksChatError::wrong_type(
                "a tool call of a Databricks stream delta",
                "an object",
            ));
        };
        let Some(function) = fields.get_mut("function") else {
            continue;
        };
        let Some(function) = as_object_mut(function) else {
            return Err(DatabricksChatError::wrong_type(
                "function of a tool call",
                "an object",
            ));
        };
        if matches!(function.get("arguments"), Some(Json::Text(text)) if text == "{}") {
            function.insert(String::from("arguments"), Json::Text(String::new()));
        }
    }
    Ok(())
}

fn as_object_mut(value: &mut Json) -> Option<&mut Object> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
