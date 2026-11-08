use thiserror::Error;

use crate::anthropic_invokes::cache_control::add_cache_control_to_content;
use crate::anthropic_results::ids::sanitize_anthropic_tool_use_id;
use crate::anthropic_results::server_result::find_server_tool_result;
use crate::arguments::json::{Json, Object};
use crate::arguments::parse::{ParseArgumentsError, parse_tool_call_arguments};
use crate::definitions::estimate::estimate_json_bytes;

const TYPE: &str = "type";
const FUNCTION: &str = "function";
const ID: &str = "id";
const NAME: &str = "name";
const ARGUMENTS: &str = "arguments";
const INPUT: &str = "input";
const TOOL_USE: &str = "tool_use";
const SERVER_TOOL_USE: &str = "server_tool_use";
const SERVER_TOOL_PREFIX: &str = "srvtoolu_";
const CONTEXT: &str = "Anthropic tool invoke";

pub const MAX_TOOL_CALLS: usize = 1024;
// the 1024 entries per list the history sanitizer and the Gemini invoke also let through
pub const MAX_SERVER_RESULTS: usize = 1024;
// the 8 MiB cap the Gemini tool invoke parser uses, applied to the server results copied into the output
pub const MAX_SERVER_RESULT_BYTES: usize = 0x0080_0000;

#[derive(Debug, Error)]
pub enum InvokeError {
    #[error(transparent)]
    Arguments(#[from] ParseArgumentsError),
    #[error("tool call field {field} must be a string")]
    Field { field: &'static str },
    #[error("the message has more than {limit} tool calls")]
    TooManyToolCalls { limit: usize },
    #[error("the {field} list has more than {limit} entries")]
    TooManyServerResults { field: &'static str, limit: usize },
    #[error("the server results to copy exceed {limit} bytes")]
    ServerResultsTooLarge { limit: usize },
}

// LiteLLM skips any tool whose type is not "function" and keeps the rest in source order
pub fn to_anthropic_tool_invokes(
    tool_calls: &[Json],
    web_search_results: &[Json],
    tool_results: &[Json],
) -> Result<Vec<Object>, InvokeError> {
    if tool_calls.len() > MAX_TOOL_CALLS {
        return Err(InvokeError::TooManyToolCalls {
            limit: MAX_TOOL_CALLS,
        });
    }
    for (field, results) in [
        ("web_search_results", web_search_results),
        ("tool_results", tool_results),
    ] {
        if results.len() > MAX_SERVER_RESULTS {
            return Err(InvokeError::TooManyServerResults {
                field,
                limit: MAX_SERVER_RESULTS,
            });
        }
    }
    let mut blocks = Vec::with_capacity(tool_calls.len());
    let mut copied_bytes: usize = 0;
    for candidate in tool_calls {
        let Some(tool) = candidate.as_object() else {
            continue;
        };
        if tool.get(TYPE).and_then(Json::as_str) != Some(FUNCTION) {
            continue;
        }
        blocks.extend(convert_tool(
            tool,
            web_search_results,
            tool_results,
            &mut copied_bytes,
        )?);
    }
    Ok(blocks)
}

fn convert_tool(
    tool: &Object,
    web_search_results: &[Json],
    tool_results: &[Json],
    copied_bytes: &mut usize,
) -> Result<Vec<Object>, InvokeError> {
    let (name, tool_input) = parse_function(tool)?;
    let Some(Json::Text(tool_id)) = tool.get(ID) else {
        return Err(InvokeError::Field { field: ID });
    };
    if tool_id.starts_with(SERVER_TOOL_PREFIX)
        && let Some(result) = find_server_tool_result(tool_id, web_search_results, tool_results)
    {
        charge_server_result(result, copied_bytes)?;
        return Ok(vec![
            Object::from_iter([
                (TYPE.to_owned(), Json::Text(SERVER_TOOL_USE.to_owned())),
                (ID.to_owned(), Json::Text(tool_id.clone())),
                (NAME.to_owned(), name),
                (INPUT.to_owned(), tool_input),
            ]),
            result.clone(),
        ]);
    }
    let element = Object::from_iter([
        (TYPE.to_owned(), Json::Text(TOOL_USE.to_owned())),
        (
            ID.to_owned(),
            Json::Text(sanitize_anthropic_tool_use_id(tool_id)),
        ),
        (NAME.to_owned(), name),
        (INPUT.to_owned(), tool_input),
    ]);
    Ok(vec![add_cache_control_to_content(element, tool)])
}

// the head is the object cost estimate_json_bytes charges, so the copy it is about to make is measured without making it
fn charge_server_result(result: &Object, copied_bytes: &mut usize) -> Result<(), InvokeError> {
    let bytes = result.iter().fold(2_usize, |total, (key, value)| {
        total
            .saturating_add(key.len().saturating_add(4))
            .saturating_add(estimate_json_bytes(value))
    });
    *copied_bytes = copied_bytes.saturating_add(bytes);
    if *copied_bytes > MAX_SERVER_RESULT_BYTES {
        return Err(InvokeError::ServerResultsTooLarge {
            limit: MAX_SERVER_RESULT_BYTES,
        });
    }
    Ok(())
}

fn parse_function(tool: &Object) -> Result<(Json, Json), InvokeError> {
    let function = match tool.get(FUNCTION) {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => return Err(InvokeError::Field { field: FUNCTION }),
    };
    let name = match function.and_then(|fields| fields.get(NAME)) {
        None | Some(Json::Null) => Json::Null,
        Some(Json::Text(text)) => Json::Text(text.clone()),
        Some(_) => return Err(InvokeError::Field { field: NAME }),
    };
    let arguments = match function.and_then(|fields| fields.get(ARGUMENTS)) {
        None | Some(Json::Null) => "",
        Some(Json::Text(text)) => text.as_str(),
        Some(_) => return Err(InvokeError::Field { field: ARGUMENTS }),
    };
    let tool_input = parse_tool_call_arguments(arguments, name.as_str(), Some(CONTEXT))?;
    Ok((name, tool_input))
}
