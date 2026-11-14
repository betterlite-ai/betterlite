use super::error::MistralFilesError;
use super::records::{FileUpload, MistralFile, MistralFileDeleted, MistralPurpose};
use crate::files::records::{FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose};
use crate::mistral_identity::base_url;
use betterlite_config::sections::mistral_resources::Settings;
use betterlite_http::url_path::encode_segment;
use betterlite_prompts::media_metadata::{FileData, extract_file_data};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUploadRequest {
    pub file: FileData,
    pub purpose: Option<String>,
}
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
    let extracted = extract_file_data(&request.file)?;
    Ok(FileUpload {
        url: format!("{}/v1/files", base_url(api_base, settings)),
        filename: extracted
            .filename
            .unwrap_or_else(|| format!("file_{now}.jsonl")),
        content_type: extracted.content_type,
        content: extracted.content,
        purpose,
    })
}
pub fn file_url(
    file_id: Option<&str>,
    suffix: &str,
    api_base: Option<&str>,
    settings: &Settings,
) -> Result<String, MistralFilesError> {
    let file_id = file_id.ok_or(MistralFilesError::FileIdRequired)?;
    let encoded = encode_segment(file_id, "file_id")?;
    Ok(format!(
        "{}/v1/files/{encoded}{suffix}",
        base_url(api_base, settings)
    ))
}
pub fn list_request(
    purpose: Option<&str>,
    api_base: Option<&str>,
    settings: &Settings,
) -> Result<(String, Vec<(String, String)>), MistralFilesError> {
    let url = format!("{}/v1/files", base_url(api_base, settings));
    let Some(purpose) = purpose.filter(|text| !text.is_empty()) else {
        return Ok((url, Vec::new()));
    };
    let mapped = upload_purpose(Some(purpose))?.wire_name();
    Ok((url, vec![(String::from("purpose"), mapped.to_owned())]))
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
pub fn to_file_deleted(deleted: &MistralFileDeleted) -> FileDeleted {
    FileDeleted {
        id: deleted.id.clone(),
        object: FileKind::File,
        deleted: deleted.deleted,
    }
}
