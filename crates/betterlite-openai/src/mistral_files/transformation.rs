use betterlite_config::sections::mistral_resources::Settings;
use betterlite_http::url_path::encode_segment;
use betterlite_prompts::media_metadata::{FileData, extract_file_data};

use crate::files::records::{FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose};
use crate::mistral_identity::base_url;

use super::error::MistralFilesError;
use super::records::{
    FileUpload, MistralFile, MistralFileDeleted, MistralFileList, MistralPurpose,
};

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["purpose"];

// LiteLLM's CreateFileRequest: the purpose is the caller's create-file purpose, before mapping
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUploadRequest {
    pub file: FileData,
    pub purpose: Option<String>,
}

#[must_use]
pub fn upload_purpose(purpose: Option<&str>) -> Result<MistralPurpose, MistralFilesError> {
    match purpose {
        Some("fine-tune") => Ok(MistralPurpose::FineTune),
        Some("batch") => Ok(MistralPurpose::Batch),
        Some("ocr" | "user_data") => Ok(MistralPurpose::Ocr),
        Some(other) => Err(MistralFilesError::UnsupportedPurpose {
            purpose: other.to_owned(),
        }),
        None => Ok(MistralPurpose::Batch),
    }
}

pub fn create_upload(
    request: &FileUploadRequest,
    now: i64,
    api_base: Option<&str>,
    settings: &Settings,
) -> Result<FileUpload, MistralFilesError> {
    let purpose = upload_purpose(request.purpose.as_deref())?;
    let extracted =
        extract_file_data(&request.file).map_err(|source| MistralFilesError::Content { source })?;
    let filename = extracted
        .filename
        .unwrap_or_else(|| format!("file_{now}.jsonl"));
    Ok(FileUpload {
        url: format!("{}/v1/files", base_url(api_base, settings)),
        filename,
        content_type: extracted.content_type,
        content: extracted.content,
        purpose,
    })
}

pub fn file_url(
    file_id: &str,
    suffix: &str,
    api_base: Option<&str>,
    settings: &Settings,
) -> Result<String, MistralFilesError> {
    let encoded =
        encode_segment(file_id, "file_id").map_err(|source| MistralFilesError::Path { source })?;
    Ok(format!(
        "{}/v1/files/{encoded}{suffix}",
        base_url(api_base, settings)
    ))
}

pub fn content_url(
    file_id: Option<&str>,
    api_base: Option<&str>,
    settings: &Settings,
) -> Result<String, MistralFilesError> {
    match file_id {
        Some(file_id) => file_url(file_id, "/content", api_base, settings),
        None => Err(MistralFilesError::FileIdRequired),
    }
}

// An absent or empty purpose sends no filter, as LiteLLM's transform does
pub fn list_request(
    purpose: Option<&str>,
    api_base: Option<&str>,
    settings: &Settings,
) -> Result<(String, Vec<(String, String)>), MistralFilesError> {
    let url = format!("{}/v1/files", base_url(api_base, settings));
    let Some(purpose) = purpose.filter(|text| !text.is_empty()) else {
        return Ok((url, Vec::new()));
    };
    let mapped = upload_purpose(Some(purpose))?;
    Ok((
        url,
        vec![(String::from("purpose"), mapped.wire_name().to_owned())],
    ))
}

#[must_use]
pub fn openai_purpose(purpose: &str) -> FilesPurpose {
    match purpose {
        "fine-tune" => FilesPurpose::FineTune,
        "batch" => FilesPurpose::Batch,
        _ => FilesPurpose::UserData,
    }
}

#[must_use]
pub fn to_file_object(file: &MistralFile, now: i64) -> FileObject {
    FileObject {
        id: file.id.clone(),
        bytes: file.bytes,
        created_at: file.created_at.unwrap_or(now),
        filename: file.filename.clone(),
        object: FileKind::File,
        purpose: openai_purpose(&file.purpose),
        status: Some(FileStatus::Uploaded),
        expires_at: file.expires_at,
        status_details: None,
    }
}

#[must_use]
pub fn to_file_objects(list: &MistralFileList, now: i64) -> Vec<FileObject> {
    let mut objects = Vec::with_capacity(list.data.len());
    for file in &list.data {
        objects.push(to_file_object(file, now));
    }
    objects
}

#[must_use]
pub fn to_file_deleted(deleted: &MistralFileDeleted) -> FileDeleted {
    FileDeleted {
        id: deleted.id.clone(),
        object: FileKind::File,
        deleted: deleted.deleted,
    }
}
