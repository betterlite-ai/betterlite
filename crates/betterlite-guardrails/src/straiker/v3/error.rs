use thiserror::Error;

#[derive(Debug, Error)]
pub enum V3Error {
    #[error("v3 payload value does not serialize to JSON")]
    Serialize(#[source] serde_json::Error),
}
