use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{FileKind, FileObject, FileStatus, FilesPurpose};
use serde::Deserialize;

use super::error::StudioFilesError;
use crate::provider_metadata::timestamp::{LocalTimeContext, decode_vertex_timestamp};

pub const METADATA_BODY_LIMIT: usize = 1024 * 1024;
const MAX_TIMESTAMP_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataShape {
    Create,
    Retrieve,
}

#[derive(Debug, Deserialize)]
struct CreateEnvelope {
    file: CreateMetadata,
}

#[derive(Debug, Deserialize)]
struct CreateMetadata {
    uri: String,
    #[serde(rename = "sizeBytes")]
    size_bytes: String,
    #[serde(rename = "createTime")]
    create_time: String,
    #[serde(rename = "displayName")]
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct RetrieveMetadata {
    #[serde(default)]
    uri: Option<String>,
    #[serde(rename = "sizeBytes", default)]
    size_bytes: Option<String>,
    #[serde(rename = "createTime")]
    create_time: String,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<Json>,
}

// The TypedDict declares sizeBytes as str and LiteLLM runs int() over it, so the text is parsed here.
fn size_bytes(text: &str) -> Result<i64, StudioFilesError> {
    text.trim()
        .parse::<i64>()
        .map_err(|source| StudioFilesError::SizeBytes { source })
}

pub fn decode_studio_file_metadata(
    bytes: &[u8],
    shape: MetadataShape,
    local: &LocalTimeContext,
) -> Result<FileObject, StudioFilesError> {
    if bytes.len() > METADATA_BODY_LIMIT {
        return Err(StudioFilesError::ResponseTooLarge {
            size: bytes.len(),
            limit: METADATA_BODY_LIMIT,
        });
    }
    let fields = match shape {
        MetadataShape::Create => create_fields(bytes, local)?,
        MetadataShape::Retrieve => retrieve_fields(bytes, local)?,
    };
    Ok(FileObject {
        id: fields.id,
        bytes: fields.bytes,
        created_at: fields.created_at,
        filename: fields.filename,
        object: FileKind::File,
        purpose: FilesPurpose::UserData,
        status: Some(fields.status),
        expires_at: None,
        status_details: fields.status_details,
    })
}

struct Fields {
    id: String,
    bytes: i64,
    created_at: i64,
    filename: String,
    status: FileStatus,
    status_details: Option<String>,
}

// A create reply wraps the record in `{"file": ...}` and requires every field the OpenAI object needs.
fn create_fields(bytes: &[u8], local: &LocalTimeContext) -> Result<Fields, StudioFilesError> {
    let envelope: CreateEnvelope =
        serde_json::from_slice(bytes).map_err(|source| StudioFilesError::Decode { source })?;
    let metadata = envelope.file;
    Ok(Fields {
        id: metadata.uri,
        bytes: size_bytes(&metadata.size_bytes)?,
        created_at: decode_created_at(&metadata.create_time, local)?,
        filename: metadata.display_name,
        status: FileStatus::Uploaded,
        status_details: None,
    })
}

// A retrieve reply is the record itself; uri, displayName and sizeBytes fall back to empty.
fn retrieve_fields(bytes: &[u8], local: &LocalTimeContext) -> Result<Fields, StudioFilesError> {
    let metadata: RetrieveMetadata =
        serde_json::from_slice(bytes).map_err(|source| StudioFilesError::Decode { source })?;
    let status = match metadata.state.as_deref() {
        Some("ACTIVE") => FileStatus::Processed,
        Some("FAILED") => FileStatus::Error,
        _ => FileStatus::Uploaded,
    };
    let size = match metadata.size_bytes.as_deref() {
        Some(text) => size_bytes(text)?,
        None => 0,
    };
    Ok(Fields {
        id: metadata.uri.unwrap_or_default(),
        bytes: size,
        created_at: decode_created_at(&metadata.create_time, local)?,
        filename: metadata.display_name.unwrap_or_default(),
        status,
        status_details: if status == FileStatus::Error {
            Some(error_details(metadata.error.as_ref())?)
        } else {
            None
        },
    })
}

// LiteLLM replaces every Z with +00:00, parses the %z suffix, then mktime drops the offset it read.
fn decode_created_at(text: &str, local: &LocalTimeContext) -> Result<i64, StudioFilesError> {
    if text.len() > MAX_TIMESTAMP_BYTES {
        return Err(StudioFilesError::TimestampTooLong {
            size: text.len(),
            limit: MAX_TIMESTAMP_BYTES,
        });
    }
    let replaced = text.replace('Z', "+00:00");
    let (civil, offset) = split_fraction(&replaced)?;
    if !valid_utc_offset(offset) {
        return Err(StudioFilesError::TimestampFormat);
    }
    // struct_time holds whole seconds, so the fraction must parse and is then dropped.
    let zoned = format!("{civil}.000000Z");
    decode_vertex_timestamp(zoned.as_bytes(), local)
        .map_err(|source| StudioFilesError::Timestamp { source })
}

fn split_fraction(text: &str) -> Result<(&str, &str), StudioFilesError> {
    let Some((civil, fraction)) = text.split_once('.') else {
        return Err(StudioFilesError::TimestampFormat);
    };
    let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=6).contains(&digits) {
        return Err(StudioFilesError::TimestampFormat);
    }
    let Some((_fraction, offset)) = fraction.split_at_checked(digits) else {
        return Err(StudioFilesError::TimestampFormat);
    };
    Ok((civil, offset))
}

// CPython's %z: [+-]HH[:]MM([:]SS(.ffffff)?)? with two-digit hours and 00-59 minutes and seconds.
fn valid_utc_offset(offset: &str) -> bool {
    let Some(rest) = offset.strip_prefix(['+', '-']) else {
        return false;
    };
    let Some((hour, after_hour)) = rest.split_at_checked(2) else {
        return false;
    };
    if !hour.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let Some((_minute, after_minute)) = minute_field(after_hour) else {
        return false;
    };
    if after_minute.is_empty() {
        return true;
    }
    let Some((_second, after_second)) = minute_field(after_minute) else {
        return false;
    };
    if after_second.is_empty() {
        return true;
    }
    let Some(fraction) = after_second.strip_prefix('.') else {
        return false;
    };
    (1..=6).contains(&fraction.len()) && fraction.bytes().all(|byte| byte.is_ascii_digit())
}

fn minute_field(text: &str) -> Option<(&str, &str)> {
    let stripped = text.strip_prefix(':').unwrap_or(text);
    let (field, rest) = stripped.split_at_checked(2)?;
    let mut bytes = field.bytes();
    let first = bytes.next()?;
    let second = bytes.next()?;
    ((b'0'..=b'5').contains(&first) && second.is_ascii_digit()).then_some((field, rest))
}

// The declared status_details is text, and betterlite writes the provider error as standard JSON.
fn error_details(error: Option<&Json>) -> Result<String, StudioFilesError> {
    let Some(value) = error else {
        return Ok(String::new());
    };
    serde_json::to_string(value).map_err(|source| StudioFilesError::Encode { source })
}
