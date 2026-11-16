use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("the model response is not a chat, Responses or Anthropic response object")]
    Response {
        #[source]
        source: serde_json::Error,
    },
    #[error("the agentic loop tools do not hold the tool_calls list the gate produced")]
    ToolCalls {
        #[source]
        source: serde_json::Error,
    },
    #[error("`{field}` must be a non-negative integer")]
    FieldType {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("a Compresr recovery value could not be encoded as JSON")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}
