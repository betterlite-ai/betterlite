use betterlite_http::proxy::UpstreamResponse;
use serde::de::DeserializeOwned;

use crate::api::batch_records::WireDecodeLimits;
use crate::files::records::{FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose};
use crate::xai_batches::transformation::raise_for_xai_status;

use super::error::XaiFilesError;
use super::records::{XaiFile, XaiFileDeleted, XaiFileList, decode_xai_upload_reply};

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
