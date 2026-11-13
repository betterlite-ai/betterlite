use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::url_path::encode_segment;
use betterlite_prompts::media_metadata::{FileData, extract_file_data};
use serde_json::{Map, Value};

use crate::xai_batches::transformation::get_xai_api_base;

use super::error::XaiFilesError;
use super::records::XaiFileList;

pub const DEFAULT_PURPOSE: &str = "batch";
pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["purpose"];

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

// LiteLLM's get_complete_url and _file_url both hang off the same origin plus /v1/files
pub fn files_url(
    api_base: Option<&str>,
    target: FilesTarget<'_>,
    settings: &Settings,
) -> Result<String, XaiFilesError> {
    let base = format!("{}/v1/files", get_xai_api_base(api_base, settings));
    match target {
        FilesTarget::Collection => Ok(base),
        FilesTarget::File(file_id) => {
            let encoded = encode_segment(file_id, "file_id")
                .map_err(|source| XaiFilesError::Path { source })?;
            Ok(format!("{base}/{encoded}"))
        }
        FilesTarget::Content(file_id) => {
            let encoded = encode_segment(file_id, "file_id")
                .map_err(|source| XaiFilesError::Path { source })?;
            Ok(format!("{base}/{encoded}/content"))
        }
    }
}

// LiteLLM's map_openai_params override returns optional_params untouched, so a non-default one is dropped
#[must_use]
pub const fn map_openai_params(
    optional_params: Map<String, Value>,
    _non_default_params: &[(String, Value)],
    _drop_params: bool,
) -> Map<String, Value> {
    // the caller's own file options ride along; "purpose" is the only advertised one
    optional_params
}

pub fn create_upload(
    file: Option<&FileData>,
    purpose: Option<&str>,
    now_epoch: i64,
) -> Result<Vec<MultipartPart>, XaiFilesError> {
    let supplied = file.ok_or(XaiFilesError::FileDataRequired)?;
    let extracted = extract_file_data(supplied)?;
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
    if page.data.is_empty() {
        return None;
    }
    Some(PageRequest {
        url: url.to_owned(),
        query: vec![("pagination_token".to_owned(), token.to_owned())],
    })
}
