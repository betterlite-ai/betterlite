use betterlite_bedrock::s3_access::TrustedModelCredentials;
use betterlite_context::cloud_storage::{
    Encoding, VERTEX_AI_MANAGED_GCS_PREFIX, decode_cloud_object_reference, encode_object_name,
};

use super::error::GcsAccessError;

pub const GCS_SCHEME: &str = "gs://";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcsObject {
    pub bucket: String,
    pub encoded_object: String,
}

pub fn parse_gcs_uri(
    file_id: &str,
    configured_bucket_name: &str,
    trusted: &TrustedModelCredentials,
) -> Result<GcsObject, GcsAccessError> {
    let allow_legacy = trusted.allows_legacy();
    let (bucket, object) = decode_cloud_object_reference(
        file_id,
        GCS_SCHEME,
        configured_bucket_name,
        &[VERTEX_AI_MANAGED_GCS_PREFIX],
        allow_legacy,
    )?;
    Ok(GcsObject {
        bucket,
        encoded_object: encode_object_name(&object, Encoding::Gcs),
    })
}
