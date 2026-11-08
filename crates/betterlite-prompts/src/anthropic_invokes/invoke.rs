use thiserror::Error;

use crate::anthropic_invokes::cache_control::add_cache_control_to_content;
use crate::anthropic_results::ids::sanitize_anthropic_tool_use_id;
use crate::anthropic_results::server_result::find_server_tool_result;
use crate::arguments::json::{Json, Object};
use crate::arguments::parse::{ParseArgumentsError, parse_tool_call_arguments};

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

#[derive(Debug, Error)]
pub enum InvokeError {
    #[error(transparent)]
    Arguments(#[from] ParseArgumentsError),
    #[error("tool call field {field} must be a string")]
    Field { field: &'static str },
}

// LiteLLM skips any tool whose type is not "function" and keeps the rest in source order
pub fn to_anthropic_tool_invokes(
    tool_calls: &[Json],
    web_search_results: &[Json],
    tool_results: &[Json],
) -> Result<Vec<Object>, InvokeError> {
    let mut blocks = Vec::with_capacity(tool_calls.len());
    for candidate in tool_calls {
        let Some(tool) = candidate.as_object() else {
            continue;
        };
        if tool.get(TYPE).and_then(Json::as_str) != Some(FUNCTION) {
            continue;
        }
        blocks.extend(convert_tool(tool, web_search_results, tool_results)?);
    }
    Ok(blocks)
}

fn convert_tool(
    tool: &Object,
    web_search_results: &[Json],
    tool_results: &[Json],
) -> Result<Vec<Object>, InvokeError> {
    let (name, tool_input) = parse_function(tool)?;
    let Some(Json::Text(tool_id)) = tool.get(ID) else {
        return Err(InvokeError::Field { field: ID });
    };
    if tool_id.starts_with(SERVER_TOOL_PREFIX)
        && let Some(result) = find_server_tool_result(tool_id, web_search_results, tool_results)
    {
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
