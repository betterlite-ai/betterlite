use betterlite_openai::files::records::{
    FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose,
};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

use crate::provider_metadata::timestamp::{LocalTimeContext, decode_vertex_timestamp};

use super::error::VertexFilesError;

pub const METADATA_BODY_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataShape {
    Upload,
    Retrieve,
}

// LiteLLM's _GcsObjectJson and its metadata record: the store's own fields, read only where it names the file
#[serde_as]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetadataRecord {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    id: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    name: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    time_created: String,
    #[serde(default)]
    purpose: Option<FilesPurpose>,
    #[serde(default)]
    metadata: Option<ObjectMetadata>,
}

#[derive(Debug, Deserialize)]
struct ObjectMetadata {
    #[serde(default)]
    purpose: Option<FilesPurpose>,
}

pub fn decode_vertex_file_metadata(
    bytes: &[u8],
    shape: MetadataShape,
    local: &LocalTimeContext,
) -> Result<FileObject, VertexFilesError> {
    if bytes.len() > METADATA_BODY_LIMIT {
        return Err(VertexFilesError::ResponseTooLarge {
            size: bytes.len(),
            limit: METADATA_BODY_LIMIT,
        });
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let record: MetadataRecord =
        serde_path_to_error::deserialize(&mut decoder).map_err(VertexFilesError::Record)?;
    decoder.end().map_err(VertexFilesError::Decode)?;
    let purpose = purpose_of(&record, shape);
    let size = match record.size {
        Some(size) => size_bytes(&size)?,
        None => 0,
    };
    let created_at = decode_vertex_timestamp(record.time_created.as_bytes(), local)
        .map_err(VertexFilesError::Timestamp)?;
    Ok(FileObject {
        id: format!("gs://{}", parent_object(&record.id)),
        bytes: size,
        created_at,
        filename: record.name,
        object: FileKind::File,
        purpose,
        status: Some(status_of(shape)),
        expires_at: None,
        status_details: None,
    })
}

// the upload reply carries the purpose at its root, the object record nests it under metadata
fn purpose_of(record: &MetadataRecord, shape: MetadataShape) -> FilesPurpose {
    let purpose = match shape {
        MetadataShape::Upload => record.purpose,
        MetadataShape::Retrieve => record
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.purpose),
    };
    purpose.unwrap_or(FilesPurpose::Batch)
}

const fn status_of(shape: MetadataShape) -> FileStatus {
    match shape {
        MetadataShape::Upload => FileStatus::Uploaded,
        MetadataShape::Retrieve => FileStatus::Processed,
    }
}

// the last slash component is a generation suffix whether or not it looks numeric
fn parent_object(id: &str) -> &str {
    id.rsplit_once('/')
        .map_or("", |(parent, _generation)| parent)
}

// the TypedDict declares size as a string and LiteLLM runs int() over it, so the text is parsed here
fn size_bytes(size: &str) -> Result<i64, VertexFilesError> {
    size.trim()
        .parse::<i64>()
        .map_err(|source| VertexFilesError::Size { source })
}

// LiteLLM takes the id from the delete request URL, which carries the bucket and the encoded object path
pub fn deleted_file(url: Option<&str>) -> FileDeleted {
    let id = url
        .and_then(storage_reference)
        .unwrap_or_else(|| "deleted".to_owned());
    FileDeleted {
        id,
        object: FileKind::File,
        deleted: true,
    }
}

fn storage_reference(url: &str) -> Option<String> {
    if !(url.contains("/b/") && url.contains("/o/")) {
        return None;
    }
    let bucket = url.rsplit("/b/").next()?.split("/o/").next()?;
    let encoded = url.rsplit("/o/").next()?.split('?').next()?;
    // urllib.parse.unquote keeps invalid UTF-8 as replacement characters
    let object = percent_decode_str(encoded).decode_utf8_lossy();
    Some(format!("gs://{bucket}/{object}"))
}

mod tests;
