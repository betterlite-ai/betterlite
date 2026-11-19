use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChatContentError {
    #[error("a translated record could not be encoded as JSON: {source}")]
    Encode {
        #[from]
        source: serde_json::Error,
    },
}
