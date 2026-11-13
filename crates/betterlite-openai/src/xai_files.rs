pub mod error;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::url_path::encode_segment;
use betterlite_prompts::media_metadata::{FileData, extract_file_data};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::api::batch_records::WireDecodeLimits;
use crate::files::records::{FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose};
use crate::xai_batches::transformation::{get_xai_api_base, raise_for_xai_status};
use error::XaiFilesError;

pub const DEFAULT_PURPOSE: &str = "batch";
pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["purpose"];

// LiteLLM's XAIFile: purpose is stored by xAI but never read, every projection reports batch
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct XaiFile {
    pub id: String,
    #[serde(default)]
    pub bytes: i64,
    pub created_at: Option<i64>,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub purpose: String,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct XaiFileList {
    #[serde(default)]
    pub data: Vec<XaiFile>,
    pub pagination_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct XaiFileDeleted {
    pub id: String,
    #[serde(default = "default_deleted")]
    pub deleted: bool,
}

const fn default_deleted() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesTarget<'id> {
    Collection,
    File(&'id str),
    Content(&'id str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub url: String,
    pub query: Vec<(String, String)>,
}

pub fn decode_xai_upload_reply<R>(
    bytes: &[u8],
    limits: &WireDecodeLimits,
) -> Result<R, XaiFilesError>
where
    R: DeserializeOwned,
{
    if bytes.len() > limits.max_json_bytes {
        return Err(XaiFilesError::TooLarge {
            limit: limits.max_json_bytes,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| XaiFilesError::Decode { source })
}

// LiteLLM's get_complete_url and _file_url both hang off the same origin plus /v1/files
pub fn files_url(
    api_base: Option<&str>,
    target: FilesTarget<'_>,
    settings: &Settings,
) -> Result<String, XaiFilesError> {
    let base = format!("{}/v1/files", get_xai_api_base(api_base, settings));
    let (file_id, suffix) = match target {
        FilesTarget::Collection => return Ok(base),
        FilesTarget::File(file_id) => (file_id, ""),
        FilesTarget::Content(file_id) => (file_id, "/content"),
    };
    let encoded =
        encode_segment(file_id, "file_id").map_err(|source| XaiFilesError::Path { source })?;
    Ok(format!("{base}/{encoded}{suffix}"))
}

pub fn create_upload(
    file: Option<&FileData>,
    purpose: Option<&str>,
    now_epoch: i64,
) -> Result<Vec<MultipartPart>, XaiFilesError> {
    let extracted = extract_file_data(file.ok_or(XaiFilesError::FileDataRequired)?)?;
    let filename = extracted
        .filename
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("file_{now_epoch}.jsonl"));
    let sent_purpose = purpose
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PURPOSE);
    Ok(vec![
        MultipartPart {
            name: "file".to_owned(),
            file_name: Some(filename),
            content_type: Some(extracted.content_type),
            data: extracted.content,
        },
        MultipartPart {
            name: "purpose".to_owned(),
            file_name: None,
            content_type: None,
            data: sent_purpose.as_bytes().to_vec(),
        },
    ])
}

// a cursor advances only when the page carries both a token and at least one file
#[must_use]
pub fn next_page(page: &XaiFileList, url: &str) -> Option<PageRequest> {
    let token = page
        .pagination_token
        .as_deref()
        .filter(|token| !token.is_empty())?;
    (!page.data.is_empty()).then(|| PageRequest {
        url: url.to_owned(),
        query: vec![("pagination_token".to_owned(), token.to_owned())],
    })
}

// xAI stores purpose as an empty string and has no file status, so LiteLLM reports both itself
#[must_use]
pub fn to_file_object(file: &XaiFile, now_epoch: i64) -> FileObject {
    FileObject {
        id: file.id.clone(),
        bytes: file.bytes,
        created_at: file.created_at.unwrap_or(now_epoch),
        filename: file.filename.clone(),
        object: FileKind::File,
        purpose: FilesPurpose::Batch,
        status: Some(FileStatus::Uploaded),
        expires_at: file.expires_at,
        status_details: None,
    }
}

#[must_use]
pub fn to_file_list(page: &XaiFileList, now_epoch: i64) -> Vec<FileObject> {
    page.data
        .iter()
        .map(|file| to_file_object(file, now_epoch))
        .collect()
}

#[must_use]
pub fn to_file_deleted(deleted: &XaiFileDeleted) -> FileDeleted {
    FileDeleted {
        id: deleted.id.clone(),
        object: FileKind::File,
        deleted: deleted.deleted,
    }
}

// LiteLLM checks the status before a file reply reaches its record, then hands the record to the projection
pub fn decode_file_reply<R, S, F>(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
    project: F,
) -> Result<S, XaiFilesError>
where
    R: DeserializeOwned,
    F: FnOnce(R) -> S,
{
    raise_for_xai_status(reply)?;
    let record: R = decode_xai_upload_reply(&reply.body, limits)?;
    Ok(project(record))
}

mod tests;
