use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyntheticEventsError {
    #[error("range() arg 3 must not be zero")]
    ZeroChunkSize,
    #[error("synthetic event payload could not be converted: {source}")]
    Payload {
        #[source]
        source: serde_json::Error,
    },
    #[error("{field} must be a JSON object")]
    NotAnObject { field: &'static str },
}

impl From<serde_json::Error> for SyntheticEventsError {
    fn from(source: serde_json::Error) -> Self {
        Self::Payload { source }
    }
}
