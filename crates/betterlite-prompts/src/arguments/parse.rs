use super::repair::attempt_json_repair;
use serde_json::{Map, Value};
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
) -> Result<Value, ParseArgumentsError> {
    if arguments.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let parse_error = match serde_json::from_str(arguments) {
        Ok(value) => return Ok(value),
        Err(source) => source,
    };
    match attempt_json_repair(arguments) {
        Some(value) => Ok(value),
        None => Err(ParseArgumentsError::Malformed {
            tool_name: tool_name.map(ToOwned::to_owned),
            context: context.map(ToOwned::to_owned),
            arguments: arguments.to_owned(),
            source: parse_error,
        }),
    }
}

mod tests;
