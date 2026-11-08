use thiserror::Error;

#[derive(Debug, Error)]
pub enum CohereError {
    #[error("the message list is empty")]
    EmptyMessages,
    #[error("messages has more than {limit} entries")]
    TooManyMessages { limit: usize },
    #[error("a message has more than {limit} content parts")]
    TooManyParts { limit: usize },
    #[error("the message list has more than {limit} tool calls")]
    TooManyToolCalls { limit: usize },
    #[error("message {index} has no content")]
    MissingContent { index: usize },
    #[error("message {index} content is a list, which the Cohere text prompt cannot carry")]
    ContentNotText { index: usize },
    #[error("message {index} content part {part} text must be a string")]
    TextPart { index: usize, part: usize },
    #[error("message {index} fields cannot be encoded for projection")]
    FieldsEncode {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("message {index} fields are not a chat message")]
    Fields {
        index: usize,
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("tool call {index} arguments are not valid JSON")]
    Arguments {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("tool call {index} has no arguments")]
    MissingArguments { index: usize },
    #[error("tool call {index} has no function")]
    MissingFunction { index: usize },
    #[error("tool call {index} has no function name")]
    MissingFunctionName { index: usize },
    #[error("message {index} is a function message without a name")]
    MissingMessageName { index: usize },
    #[error("message {index} has role {role}, which the Cohere history cannot place")]
    UnsupportedRole { index: usize, role: String },
}
