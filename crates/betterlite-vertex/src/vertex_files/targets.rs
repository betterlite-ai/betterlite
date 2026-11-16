use betterlite_bedrock::s3_access::TrustedModelCredentials;
use betterlite_context::cloud_storage::{Encoding, encode_object_name, split_bucket_name};
use betterlite_openai::files::records::CreatePurpose;
use uuid::Uuid;

use crate::vertex_gcs_access::bucket::{BucketCandidates, configured_bucket_name};
use crate::vertex_gcs_access::error::GcsAccessError;
use crate::vertex_gcs_access::parse::parse_gcs_uri;

use super::FileRequest;
use super::error::VertexFilesError;
use super::naming;

pub const STORAGE_BASE_URL: &str = "https://storage.googleapis.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadTarget {
    Metadata,
    Content,
}

// the batch custom endpoint has no batch surface, so it is refused before the bucket is even resolved
pub fn upload_url(
    request: &FileRequest<'_>,
    api_base: Option<&str>,
    candidates: &BucketCandidates,
    id: Uuid,
) -> Result<String, VertexFilesError> {
    if request.purpose == CreatePurpose::Batch && request.custom_endpoint {
        return Err(VertexFilesError::CustomEndpointBatch);
    }
    let configured = configured_bucket_name(candidates)?;
    let (bucket, object_prefix) = split_bucket_name(&configured).map_err(GcsAccessError::from)?;
    let base_name = naming::get_object_name(request, id)?;
    let object_name = if object_prefix.is_empty() {
        base_name
    } else {
        format!("{object_prefix}/{base_name}")
    };
    let encoded = encode_object_name(&object_name, Encoding::Gcs);
    let endpoint = format!("upload/storage/v1/b/{bucket}/o?uploadType=media&name={encoded}");
    let base_url = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(STORAGE_BASE_URL);
    Ok(format!("{base_url}/{endpoint}"))
}

// retrieve, delete and content reads always reach the fixed storage host, even when the upload used a custom base
pub fn read_url(
    file_id: &str,
    candidates: &BucketCandidates,
    trusted: &TrustedModelCredentials,
    target: ReadTarget,
) -> Result<String, VertexFilesError> {
    let configured = configured_bucket_name(candidates)?;
    let object = parse_gcs_uri(file_id, &configured, trusted)?;
    let suffix = match target {
        ReadTarget::Metadata => "",
        ReadTarget::Content => "?alt=media",
    };
    Ok(format!(
        "{STORAGE_BASE_URL}/storage/v1/b/{}/o/{}{suffix}",
        object.bucket, object.encoded_object
    ))
}

mod tests;
