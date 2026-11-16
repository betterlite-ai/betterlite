use betterlite_prompts::arguments::parse::ParseArgumentsError;
use thiserror::Error;

pub const ARGUMENT_CONTEXT: &str = "Anthropic pass-through adapter";

#[derive(Debug, Error)]
pub enum ChatOutputError {
    #[error(
        "Failed to parse tool call arguments for tool '{tool_name}' ({context}). Error: {source}. Arguments: {arguments}"
    )]
    Arguments {
        tool_name: String,
        context: &'static str,
        arguments: String,
        #[source]
        source: Box<ParseArgumentsError>,
    },
    #[error("tool call arguments for tool {tool_name:?} decoded to a non-object JSON value")]
    NonObjectArguments { tool_name: String },
    #[error("thinking block field {field} is not a string")]
    NonTextThinkingField { field: &'static str },
    #[error("a streaming chunk has no choices to translate")]
    NoChoices,
}
