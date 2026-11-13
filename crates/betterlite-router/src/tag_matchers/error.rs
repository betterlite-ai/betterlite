use thiserror::Error;

#[derive(Debug, Error)]
pub enum TagMatcherError {
    #[error("the tag match document is {bytes} bytes, past the {limit} byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the tag match document is not a valid request")]
    Invalid {
        #[source]
        source: serde_json::Error,
    },
}
