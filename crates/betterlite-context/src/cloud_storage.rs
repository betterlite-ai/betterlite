use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, percent_encode};
use thiserror::Error;

pub mod policy;

const URL_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');
const S3_ENCODE_SET: &AsciiSet = &URL_ENCODE_SET.remove(b'/');

#[derive(Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct CloudStorageError {
    message: String,
}

impl CloudStorageError {
    fn new(message: &str) -> Self {
        let owned = message.to_owned();
        Self { message: owned }
    }
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
    let component = replaced.rsplit('/').next().unwrap_or_default().trim();
    if component.is_empty() || component == "." || component == ".." {
        return fallback.to_owned();
    }
    let mut sanitized = String::with_capacity(component.len());
    let mut outside = false;
    for character in component.chars() {
        let control = character.is_ascii_control();
        let allowed = character.is_ascii_alphanumeric() || "._-".contains(character);
        match (control, allowed, outside) {
            (true, _, _) => {
                sanitized.push('_');
                outside = false;
            }
            (false, true, _) => {
                sanitized.push(character);
                outside = false;
            }
            (false, false, true) => {}
            (false, false, false) => {
                sanitized.push('_');
                outside = true;
            }
        }
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
    let mut segments = Vec::new();
    for segment in value.replace('\\', "/").split('/') {
        let safe = sanitize_component(Some(segment), "");
        if !safe.is_empty() {
            segments.push(safe);
        }
    }
    if segments.is_empty() {
        fallback.to_owned()
    } else {
        segments.join("/")
    }
}

#[must_use]
pub fn build_object_name(prefix: &str, filename: Option<&str>, fallback: &str) -> String {
    let safe = sanitize_component(filename, fallback);
    format!("{prefix}{}-{safe}", uuid::Uuid::new_v4().simple())
}

pub fn validate_cloud_object_path(object_name: &str) -> Result<(), CloudStorageError> {
    if object_name.is_empty() {
        return Err(CloudStorageError::new(
            "Cloud storage object name is required",
        ));
    }
    if object_name.starts_with('/') {
        return Err(CloudStorageError::new(
            "Cloud storage object name must be relative",
        ));
    }
    if object_name
        .chars()
        .any(|character| character.is_ascii_control())
    {
        return Err(CloudStorageError::new(
            "Cloud storage object name contains control characters",
        ));
    }
    let segments: Vec<&str> = object_name.split('/').collect();
    let has_empty_parent = segments
        .iter()
        .take(segments.len().saturating_sub(1))
        .any(|segment| segment.is_empty());
    if segments
        .iter()
        .any(|segment| *segment == "." || *segment == "..")
        || has_empty_parent
    {
        return Err(CloudStorageError::new(
            "Cloud storage object name contains an invalid path segment",
        ));
    }
    Ok(())
}

pub fn split_bucket_name(input: &str) -> Result<(String, String), CloudStorageError> {
    let value = input.trim();
    if value.is_empty() {
        return Err(CloudStorageError::new(
            "Cloud storage bucket name is required",
        ));
    }
    if value.contains("://") || value.contains('?') || value.contains('#') {
        return Err(CloudStorageError::new(
            "Cloud storage bucket name must not include a URI scheme or query",
        ));
    }
    if value.chars().any(|character| character.is_ascii_control()) {
        return Err(CloudStorageError::new(
            "Cloud storage bucket name contains control characters",
        ));
    }
    let (bucket, raw_prefix) = value
        .split_once('/')
        .map_or((value, ""), |(bucket, prefix)| (bucket, prefix));
    if bucket.is_empty() {
        return Err(CloudStorageError::new(
            "Cloud storage bucket name is required",
        ));
    }
    if bucket.contains('\\') {
        return Err(CloudStorageError::new(
            "Cloud storage bucket name contains an invalid separator",
        ));
    }
    let prefix = raw_prefix.trim_matches('/');
    if !prefix.is_empty() {
        validate_cloud_object_path(prefix)?;
    }
    Ok((bucket.to_owned(), prefix.to_owned()))
}

#[must_use]
pub fn encode_gcs_object_name_for_url(object_name: &str) -> String {
    let mut encoded = encode_object_name(object_name, false);
    encoded.shrink_to_fit();
    encoded
}
#[must_use]
pub fn encode_s3_object_key_for_url(object_name: &str) -> String {
    let mut encoded = encode_object_name(object_name, true);
    encoded.shrink_to_fit();
    encoded
}

fn encode_object_name(object_name: &str, preserve_slash: bool) -> String {
    let decoded = percent_decode_str(object_name).decode_utf8_lossy();
    if preserve_slash {
        percent_encode(decoded.as_bytes(), S3_ENCODE_SET).to_string()
    } else {
        percent_encode(decoded.as_bytes(), URL_ENCODE_SET).to_string()
    }
}

pub fn validate_managed_cloud_file_id(
    file_id: &str,
    scheme: &str,
    configured_bucket_name: &str,
    allowed_object_prefixes: &[&str],
    allow_legacy: bool,
) -> Result<(String, String), CloudStorageError> {
    let decoded = percent_decode_str(file_id).decode_utf8_lossy();
    if !decoded.starts_with(scheme) {
        return Err(CloudStorageError::new(&format!(
            "file_id must be a {scheme} URI"
        )));
    }
    let path = decoded.strip_prefix(scheme).unwrap_or_default();
    let (bucket, object) = path.split_once('/').ok_or_else(|| {
        CloudStorageError::new("file_id must include a cloud storage object name")
    })?;
    let (configured_bucket, configured_prefix) = split_bucket_name(configured_bucket_name)?;
    if bucket != configured_bucket {
        return Err(CloudStorageError::new(
            "file_id bucket does not match the configured storage bucket",
        ));
    }
    validate_cloud_object_path(object)?;
    let prefixes = managed_prefixes(&configured_prefix, allowed_object_prefixes);
    if prefixes.iter().any(|prefix| object.starts_with(prefix)) {
        return Ok((bucket.to_owned(), object.to_owned()));
    }
    if allow_legacy
        && (configured_prefix.is_empty() || object.starts_with(&format!("{configured_prefix}/")))
    {
        return Ok((bucket.to_owned(), object.to_owned()));
    }
    if allow_legacy {
        return Err(CloudStorageError::new(
            "file_id object does not match the configured storage prefix",
        ));
    }
    Err(CloudStorageError::new(
        "file_id must reference a LiteLLM-managed storage object",
    ))
}

fn managed_prefixes(configured_prefix: &str, allowed: &[&str]) -> Vec<String> {
    let mut prefixes: Vec<String> = allowed
        .iter()
        .map(|prefix| {
            if configured_prefix.is_empty() {
                (*prefix).to_owned()
            } else {
                format!("{configured_prefix}/{prefix}")
            }
        })
        .collect();
    prefixes.shrink_to_fit();
    prefixes
}

pub fn decode_cloud_object_reference(value: &str) -> Result<(String, String), CloudStorageError> {
    let decoded = percent_decode_str(value).decode_utf8_lossy();
    let (scheme, path) = if let Some(path) = decoded.strip_prefix("s3://") {
        ("s3://", path)
    } else if let Some(path) = decoded.strip_prefix("gs://") {
        ("gs://", path)
    } else {
        return Err(CloudStorageError::new(
            "cloud object reference must use s3:// or gs://",
        ));
    };
    let (bucket, object) = path.split_once('/').ok_or_else(|| {
        CloudStorageError::new("cloud object reference must include a bucket and object")
    })?;
    validate_cloud_object_path(object)?;
    Ok((format!("{scheme}{bucket}"), object.to_owned()))
}

mod tests;
