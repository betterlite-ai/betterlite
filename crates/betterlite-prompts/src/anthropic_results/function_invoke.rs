use thiserror::Error;

use crate::arguments::json::{Json, Object};
use crate::arguments::parse::{ParseArgumentsError, parse_tool_call_arguments};

const NAME: &str = "name";
const ARGUMENTS: &str = "arguments";
const TYPE: &str = "type";
const ID: &str = "id";
const INPUT: &str = "input";
const TOOL_USE: &str = "tool_use";
const CONTEXT: &str = "Anthropic function to tool invoke";

#[derive(Debug, Error)]
pub enum InvokeError {
    #[error(transparent)]
    Arguments(#[from] ParseArgumentsError),
    #[error("function call field {field} must be a string")]
    Field { field: &'static str },
}

pub fn to_anthropic_tool_use<Ids>(
    function_call: &Object,
    new_id: &Ids,
) -> Result<Vec<Object>, InvokeError>
where
    Ids: Fn() -> String,
{
    let name = match function_call.get(NAME) {
        None | Some(Json::Null) => "",
        Some(Json::Text(text)) => text.as_str(),
        Some(_) => return Err(InvokeError::Field { field: NAME }),
    };
    let arguments = match function_call.get(ARGUMENTS) {
        None | Some(Json::Null) => "",
        Some(Json::Text(text)) => text.as_str(),
        Some(_) => return Err(InvokeError::Field { field: ARGUMENTS }),
    };
    let tool_input = parse_tool_call_arguments(arguments, Some(name), Some(CONTEXT))?;
    Ok(vec![Object::from_iter([
        (TYPE.to_owned(), Json::Text(TOOL_USE.to_owned())),
        (ID.to_owned(), Json::Text(new_id())),
        (NAME.to_owned(), Json::Text(name.to_owned())),
        (INPUT.to_owned(), tool_input),
    ])])
}
