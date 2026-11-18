use thiserror::Error;

#[derive(Debug, Error)]
pub enum SensitiveRoutingError {
    #[error("metadata must be a JSON object to record the sensitive data routing override")]
    MetadataNotObject,
}
