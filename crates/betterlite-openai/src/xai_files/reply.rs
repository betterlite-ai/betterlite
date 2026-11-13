use betterlite_http::proxy::UpstreamResponse;

use crate::api::batch_records::WireDecodeLimits;
use crate::files::records::{FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose};
use crate::xai_batches::transformation::raise_for_xai_status;

use super::error::XaiFilesError;
use super::records::{XaiFile, XaiFileDeleted, XaiFileList, decode_xai_upload_reply};
use super::request::{PageRequest, next_page};

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

pub fn decode_file_object(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
    now_epoch: i64,
) -> Result<FileObject, XaiFilesError> {
    raise_for_xai_status(reply)?;
    let file: XaiFile = decode_xai_upload_reply(&reply.body, limits)?;
    Ok(to_file_object(&file, now_epoch))
}

pub fn decode_file_list(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
    now_epoch: i64,
) -> Result<Vec<FileObject>, XaiFilesError> {
    raise_for_xai_status(reply)?;
    let page: XaiFileList = decode_xai_upload_reply(&reply.body, limits)?;
    Ok(to_file_list(&page, now_epoch))
}

pub fn decode_file_deleted(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
) -> Result<FileDeleted, XaiFilesError> {
    raise_for_xai_status(reply)?;
    let deleted: XaiFileDeleted = decode_xai_upload_reply(&reply.body, limits)?;
    Ok(to_file_deleted(&deleted))
}

// LiteLLM reads the cursor from the page it already has, so a non-2xx reply simply carries no next page
pub fn decode_next_page(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
    url: &str,
) -> Result<Option<PageRequest>, XaiFilesError> {
    let page: XaiFileList = decode_xai_upload_reply(&reply.body, limits)?;
    Ok(next_page(&page, url))
}
