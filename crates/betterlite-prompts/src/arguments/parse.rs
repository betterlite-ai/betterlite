use super::json::{Json, Object};
use super::repair::attempt_json_repair;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseArgumentsError {
    #[error("failed to parse tool call arguments: {source}")]
    Malformed {
        tool_name: Option<String>,
        context: Option<String>,
        arguments: String,
        #[source]
        source: serde_json::Error,
    },
}

pub fn parse_tool_call_arguments(
    arguments: &str,
    tool_name: Option<&str>,
    context: Option<&str>,
) -> Result<Json, ParseArgumentsError> {
    if arguments.trim().is_empty() {
        return Ok(Json::Object(Object::new()));
    }
    let parse_error = match serde_json::from_str(arguments) {
        Ok(value) => return Ok(value),
        Err(source) => source,
    };
    attempt_json_repair(arguments).map_or_else(
        || {
            Err(ParseArgumentsError::Malformed {
                tool_name: tool_name.map(ToOwned::to_owned),
                context: context.map(ToOwned::to_owned),
                arguments: arguments.to_owned(),
                source: parse_error,
            })
        },
        Ok,
    )
}

mod tests;
