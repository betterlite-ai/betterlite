use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeminiToolResultsError {
    #[error("the message is {length} bytes, over the {limit} byte limit")]
    MessageTooLarge { length: usize, limit: usize },
    #[error("the bytes are not an OpenAI chat message")]
    MessageDecode {
        #[source]
        source: serde_json::Error,
    },
    #[error("a tool result message has role tool or function, not {role}")]
    Role { role: String },
    #[error("a content list exceeds the maximum of {limit} parts")]
    TooManyParts { limit: usize },
    #[error("the previous assistant message holds more than {limit} tool calls")]
    TooManyToolCalls { limit: usize },
    #[error(
        "a tool result message needs the name of the tool call it answers, and neither the message nor the previous tool call gives one"
    )]
    MissingName,
    #[error("field {field} must be {expected}")]
    Field {
        field: String,
        expected: &'static str,
    },
}
