#[derive(Debug, thiserror::Error)]
pub enum BlockFramingError {
    #[error("a block frame could not be written as JSON")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}
