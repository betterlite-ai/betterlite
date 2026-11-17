use betterlite_models::decisions::Json;
use tracing::debug;

use crate::api::chat_response_stream_tools::fields::string_value;
use crate::api::responses_tool_namespaces::values::{Fields, has_value};

use super::error::FileSearchToolsError;
use super::results::{NO_DEFAULT, field};

pub const STREAM_DISABLED_MESSAGE: &str =
    "Streaming is not yet supported for emulated file_search. Disabling stream for this request.";
const RESULTS_INCLUDE: &str = "file_search_call.results";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCall {
    pub include_search_results: bool,
    pub updated_kwargs: Fields,
}

// a falsey stream stays untouched; a truthy one is logged and turned off on a shallow copy of the kwargs
pub fn prepare_file_search_call(kwargs: &Fields) -> Result<PreparedCall, FileSearchToolsError> {
    let include_search_results = match kwargs.get("include") {
        None | Some(Json::Null) => false,
        Some(Json::List(items)) => items
            .iter()
            .any(|item| matches!(item, Json::Text(text) if text == RESULTS_INCLUDE)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(FileSearchToolsError::WrongType {
                field: "include",
                expected: "a list of strings",
            });
        }
    };
    let mut updated_kwargs = kwargs.clone();
    if kwargs.get("stream").is_some_and(has_value) {
        debug!("{STREAM_DISABLED_MESSAGE}");
        updated_kwargs.insert("stream".to_owned(), Json::Bool(false));
    }
    Ok(PreparedCall {
        include_search_results,
        updated_kwargs,
    })
}

// tool_call.get("call_id") or tool_call.get("id") or the fallback, with str() applied to the chosen value
pub fn extract_tool_call_fields(
    tool_call: &Fields,
    fallback_call_id: &str,
) -> Result<(String, Json), FileSearchToolsError> {
    let raw_id = [
        field(tool_call, "call_id", &NO_DEFAULT),
        field(tool_call, "id", &NO_DEFAULT),
    ]
    .into_iter()
    .find(has_value);
    let call_id = match raw_id {
        Some(value) => value_text(&value, "call_id")?,
        None => fallback_call_id.to_owned(),
    };
    let arguments = field(tool_call, "arguments", &NO_DEFAULT);
    let raw_arguments = if has_value(&arguments) {
        arguments
    } else {
        Json::Text("{}".to_owned())
    };
    Ok((call_id, raw_arguments))
}

// args.get("queries") first, then the backward-compatible "query", then the request input as one query
pub fn resolve_queries_from_args(
    args: &Fields,
    input: &Json,
) -> Result<Vec<String>, FileSearchToolsError> {
    let queries = field(args, "queries", &NO_DEFAULT);
    if !has_value(&queries) {
        let single = field(args, "query", &NO_DEFAULT);
        return if has_value(&single) {
            Ok(vec![value_text(&single, "query")?])
        } else {
            Ok(vec![string_value(input)])
        };
    }
    match queries {
        Json::List(items) => items
            .iter()
            .map(|item| value_text(item, "queries"))
            .collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Ok(vec![value_text(&queries, "queries")?])
        }
    }
}

// str() over the values a model can put in a call argument: text stays, a number reads as its JSON text
fn value_text(value: &Json, field: &'static str) -> Result<String, FileSearchToolsError> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Number(number) => Ok(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => {
            Err(FileSearchToolsError::WrongType {
                field,
                expected: "a string or a number",
            })
        }
    }
}
