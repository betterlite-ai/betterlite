use crate::s3_cache::entry::CachedEntry;

pub fn decode_body(bytes: &[u8]) -> Result<CachedEntry, DecodeError> {
    match serde_json::from_slice(bytes) {
        Ok(value) => Ok(value),
        Err(source) => Err(DecodeError::Json { source }),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid JSON cache body")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}
