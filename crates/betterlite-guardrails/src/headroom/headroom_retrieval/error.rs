use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum RetrievalError {
    #[error("the model response is not a chat, Responses or Anthropic response object")]
    Response {
        #[source]
        source: serde_json::Error,
    },
    #[error("the agentic loop tools do not hold the tool_calls list the gate produced")]
    Tools {
        #[source]
        source: serde_json::Error,
    },
    #[error("`{0}` must be an integer")]
    FieldType(&'static str),
    #[error("a Headroom agentic loop value could not be encoded as JSON")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}
