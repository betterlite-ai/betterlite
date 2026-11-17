use thiserror::Error;

#[derive(Debug, Error)]
pub enum TinyfishError {
    #[error("the TinyFish run could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
