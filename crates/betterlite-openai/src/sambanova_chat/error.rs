use thiserror::Error;

#[derive(Debug, Error)]
pub enum SambaNovaChatError {
    #[error("a chat message could not be converted for the prompt helpers: {source}")]
    Message {
        #[source]
        source: serde_json::Error,
    },
}
