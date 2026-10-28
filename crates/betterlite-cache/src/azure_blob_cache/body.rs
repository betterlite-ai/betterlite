use crate::s3_cache::entry::CachedEntry;

pub fn decode_body(bytes: &[u8]) -> Result<CachedEntry, DecodeError> {
    serde_json::from_slice(bytes).map_err(|source| DecodeError::Json { source })
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid JSON cache body")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}
