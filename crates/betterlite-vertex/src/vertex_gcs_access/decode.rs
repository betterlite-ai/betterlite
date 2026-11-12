use std::str::from_utf8;

use betterlite_bedrock::s3_access::TrustedModelCredentials;

use super::error::GcsAccessError;
use super::parse::{GcsObject, parse_gcs_uri};

pub const MAX_FILE_ID_BYTES: usize = 8192;

pub fn decode_vertex_gcs_id(
    bytes: &[u8],
    configured_bucket_name: &str,
    trusted: &TrustedModelCredentials,
) -> Result<GcsObject, GcsAccessError> {
    let limit = MAX_FILE_ID_BYTES;
    if bytes.len() > limit {
        return Err(GcsAccessError::FileIdTooLong { limit });
    }
    let file_id = from_utf8(bytes).map_err(GcsAccessError::FileIdNotUtf8)?;
    parse_gcs_uri(file_id, configured_bucket_name, trusted)
}
