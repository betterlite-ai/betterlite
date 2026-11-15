use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompletionError {
    #[error("a usage count difference does not fit in 64 bits")]
    UsageOverflow,
    #[error("the toolUse input is not JSON")]
    ToolInput {
        #[source]
        source: serde_json::Error,
    },
}
