use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeminiToolInvokesError {
    #[error("the message is {length} bytes, over the {limit} byte limit")]
    MessageTooLarge { length: usize, limit: usize },
    #[error("the bytes are not an OpenAI chat message")]
    MessageDecode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the message has more than {limit} tool calls")]
    TooManyToolCalls { limit: usize },
    #[error("the message fields cannot be encoded for projection")]
    FieldsEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("{source}")]
    Fields {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("tool call arguments are not valid JSON: {source}")]
    Arguments {
        #[source]
        source: serde_json::Error,
    },
}
