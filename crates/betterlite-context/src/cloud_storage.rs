use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, percent_encode};
use thiserror::Error;
pub mod policy;
pub const VERTEX_AI_MANAGED_GCS_PREFIX: &str = "litellm-vertex-files/";
pub const BEDROCK_MANAGED_S3_PREFIXES: [&str; 3] = [
    "litellm-bedrock-files-",
    "litellm-bedrock-files/",
    "litellm-batch-outputs/",
];
const URL_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
const S3_ENCODE_SET: &AsciiSet = &URL_ENCODE_SET.remove(b'/');
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CloudStorageError {
    #[error("Cloud storage object name is required")]
    EmptyObject,
    #[error("Cloud storage object name must be relative")]
    Absolute,
    #[error("Cloud storage object name contains control characters")]
    ObjectControl,
    #[error("Cloud storage object name contains an invalid path segment")]
    Segment,
    #[error("Cloud storage bucket name is required")]
    EmptyBucket,
    #[error("Cloud storage bucket name must not include a URI scheme or query")]
    BucketUri,
    #[error("Cloud storage bucket name contains control characters")]
    BucketControl,
    #[error("Cloud storage bucket name contains an invalid separator")]
    Separator,
    #[error("file_id must be a {0} URI")]
    Scheme(String),
    #[error("file_id must include a cloud storage object name")]
    MissingObject,
    #[error("file_id bucket does not match the configured storage bucket")]
    BucketMismatch,
    #[error("file_id object does not match the configured storage prefix")]
    PrefixMismatch,
    #[error("file_id must reference a LiteLLM-managed storage object")]
    Unmanaged,
}
#[must_use]
pub fn is_managed_cloud_storage_uri(file_id: &str) -> bool {
    let s3 = file_id.starts_with("s3://");
    let gcs = file_id.starts_with("gs://");
    s3 || gcs
}
#[must_use]
pub fn sanitize_component(input: Option<&str>, fallback: &str) -> String {
    let Some(value) = input else {
        return fallback.to_owned();
    };
    let replaced = value.replace('\\', "/");
    let component = replaced.rsplit('/').next().unwrap_or_default();
    let mut sanitized = String::with_capacity(component.len());
    let mut outside = false;
    for raw in component.chars() {
        let character = if raw.is_ascii_control() { '_' } else { raw };
        let allowed = character.is_ascii_alphanumeric() || "._-".contains(character);
        if allowed || !outside {
            sanitized.push(if allowed { character } else { '_' });
        }
        outside = !allowed;
    }
    let trimmed = sanitized.trim_matches(['.', '_']);
    if trimmed.is_empty() {
        return fallback.to_owned();
    }
    trimmed.chars().take(255).collect()
}
#[must_use]
pub fn sanitize_path(input: Option<&str>, fallback: &str) -> String {
    let Some(value) = input else {
        return fallback.to_owned();
    };
    let segments: Vec<_> = value
        .replace('\\', "/")
        .split('/')
        .map(|segment| sanitize_component(Some(segment), ""))
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        fallback.to_owned()
    } else {
        segments.join("/")
    }
}
#[must_use]
pub fn build_object_name(
    prefix: &str,
    filename: Option<&str>,
    fallback: &str,
    id: uuid::Uuid,
) -> String {
    let safe = sanitize_component(filename, fallback);
    format!("{prefix}{}-{safe}", id.simple())
}
pub fn validate_cloud_object_path(object_name: &str) -> Result<(), CloudStorageError> {
    if object_name.is_empty() {
        return Err(CloudStorageError::EmptyObject);
    }
    if object_name.starts_with('/') {
        return Err(CloudStorageError::Absolute);
    }
    if object_name.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(CloudStorageError::ObjectControl);
    }
    let mut segments = object_name.split('/').peekable();
    while let Some(segment) = segments.next() {
        if segment == "." || segment == ".." || (segment.is_empty() && segments.peek().is_some()) {
            return Err(CloudStorageError::Segment);
        }
    }
    Ok(())
}
pub fn split_bucket_name(input: &str) -> Result<(String, String), CloudStorageError> {
    let value = input.trim_matches(|character: char| {
        character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
    });
    if value.is_empty() {
        return Err(CloudStorageError::EmptyBucket);
    }
    if value.contains("://") || value.contains('?') || value.contains('#') {
        return Err(CloudStorageError::BucketUri);
    }
    if value.chars().any(|character| character.is_ascii_control()) {
        return Err(CloudStorageError::BucketControl);
    }
    let (bucket, raw_prefix) = value.split_once('/').unwrap_or((value, ""));
    if bucket.is_empty() {
        return Err(CloudStorageError::EmptyBucket);
    }
    if bucket.contains('\\') {
        return Err(CloudStorageError::Separator);
    }
    let prefix = raw_prefix.trim_matches('/');
    if !prefix.is_empty() {
        validate_cloud_object_path(prefix)?;
    }
    Ok((bucket.to_owned(), prefix.to_owned()))
}
#[derive(Clone, Copy)]
pub enum Encoding {
    Gcs,
    S3,
}
#[must_use]
pub fn encode_object_name(object_name: &str, encoding: Encoding) -> String {
    let decoded = percent_decode_str(object_name).decode_utf8_lossy();
    let set = match encoding {
        Encoding::Gcs => URL_ENCODE_SET,
        Encoding::S3 => S3_ENCODE_SET,
    };
    percent_encode(decoded.as_bytes(), set).to_string()
}
pub fn decode_cloud_object_reference(
    file_id: &str,
    scheme: &str,
    configured_bucket_name: &str,
    allowed_object_prefixes: &[&str],
    allow_legacy: bool,
) -> Result<(String, String), CloudStorageError> {
    let decoded = percent_decode_str(file_id).decode_utf8_lossy();
    let path = decoded
        .strip_prefix(scheme)
        .ok_or_else(|| CloudStorageError::Scheme(scheme.to_owned()))?;
    let (bucket, object) = path
        .split_once('/')
        .ok_or(CloudStorageError::MissingObject)?;
    let (configured_bucket, configured_prefix) = split_bucket_name(configured_bucket_name)?;
    if bucket != configured_bucket {
        return Err(CloudStorageError::BucketMismatch);
    }
    validate_cloud_object_path(object)?;
    let root = if configured_prefix.is_empty() {
        String::new()
    } else {
        format!("{configured_prefix}/")
    };
    let managed = allowed_object_prefixes
        .iter()
        .any(|prefix| object.starts_with(&format!("{root}{prefix}")));
    if managed || (allow_legacy && object.starts_with(&root)) {
        Ok((bucket.to_owned(), object.to_owned()))
    } else if allow_legacy {
        Err(CloudStorageError::PrefixMismatch)
    } else {
        Err(CloudStorageError::Unmanaged)
    }
}
mod tests;
