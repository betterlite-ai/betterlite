use std::convert::Infallible;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_file_search_tools::tools::FUNCTION_TOOL_NAME;
use crate::api::responses_tool_namespaces::values::Fields;

use super::error::FileSearchLoopError;
// ast-grep-ignore: no-long-names
use super::{DecodeError, decode_responses_file_search_args};

const FUNCTION_CALL_TYPE: &str = "function_call";
const NAME_FIELD: &str = "name";
const TYPE_FIELD: &str = "type";

// json.loads(raw_args) if isinstance(raw_args, str) else raw_args, with JSONDecodeError read as empty arguments
pub fn tool_call_arguments(
    raw: &Json,
    limits: &ApiLimits,
) -> Result<Fields, FileSearchLoopError<Infallible>> {
    match raw {
        Json::Text(text) => arguments_from_text(text, limits),
        Json::Object(fields) => Ok(fields.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            Err(FileSearchLoopError::ArgumentsNotObject)
        }
    }
}

fn arguments_from_text(
    text: &str,
    limits: &ApiLimits,
) -> Result<Fields, FileSearchLoopError<Infallible>> {
    // ast-grep-ignore: no-long-names
    match decode_responses_file_search_args(text.as_bytes(), limits) {
        Ok(fields) => Ok(fields),
        Err(DecodeError::Json(_)) => Ok(Fields::new()),
        Err(DecodeError::Limit(limit)) => Err(FileSearchLoopError::Limit(limit)),
        Err(DecodeError::NotObject) => Err(FileSearchLoopError::ArgumentsNotObject),
    }
}

// the first response's output items whose type is function_call and whose name is the file search function
pub fn file_search_calls(
    output: &[ResponseOutputItem],
) -> Result<Vec<Fields>, FileSearchLoopError<Infallible>> {
    let mut calls = Vec::new();
    for item in output {
        let text =
            serde_json::to_string(item).map_err(|source| FileSearchLoopError::Wire { source })?;
        let value: Json =
            serde_json::from_str(&text).map_err(|source| FileSearchLoopError::Wire { source })?;
        let Json::Object(fields) = value else {
            continue;
        };
        if is_file_search_call(&fields) {
            calls.push(fields);
        }
    }
    Ok(calls)
}

fn is_file_search_call(fields: &Fields) -> bool {
    matches!(fields.get(TYPE_FIELD), Some(Json::Text(kind)) if kind == FUNCTION_CALL_TYPE)
        && matches!(fields.get(NAME_FIELD), Some(Json::Text(name)) if name == FUNCTION_TOOL_NAME)
}
