use super::error::GcsCacheError;
use crate::s3_cache::entry::CachedEntry;

pub fn decode_body(bytes: &[u8]) -> Result<CachedEntry, GcsCacheError> {
    serde_json::from_str(
        std::str::from_utf8(bytes).map_err(|source| GcsCacheError::Utf8 { source })?,
    )
    .map_err(|source| GcsCacheError::Json { source })
}
