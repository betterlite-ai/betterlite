use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::{
    TwoStepFileUploadConfig, TwoStepFileUploadRequest, UploadData, UploadUrlLocation,
};
use betterlite_prompts::media_metadata::{ExtractedFileData, FileData, extract_file_data};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::StudioFilesError;
use super::{RequestParams, resolve_base, resolve_key};
use crate::studio_identity::{API_VERSION, Headers};

const UPLOAD_URL_HEADER: &str = "X-Goog-Upload-URL";

// LiteLLM writes x-goog-api-key over the caller's map and leaves every other header as sent.
pub fn validate_environment(
    caller: &BTreeMap<String, String>,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<Headers, StudioFilesError> {
    let key = resolve_key(&[api_key], settings).ok_or(StudioFilesError::MissingSettingsKey)?;
    let mut headers: Headers = caller
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect();
    headers.insert("x-goog-api-key".to_owned(), key);
    Ok(headers)
}

// The upload key may also come from litellm_params, and the base is used exactly as resolved.
pub fn complete_upload_url(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    params: &RequestParams,
    settings: &Settings,
) -> Result<String, StudioFilesError> {
    let base = resolve_base(api_base, settings);
    let candidates = [api_key, params.api_key.as_ref()];
    if resolve_key(&candidates, settings).is_none() {
        return Err(StudioFilesError::MissingApiKey);
    }
    Ok(format!("{base}/upload/{API_VERSION}/files"))
}

pub fn create_file_plan(
    file: Option<&FileData>,
    upload_url: &str,
    now_epoch: i64,
) -> Result<TwoStepFileUploadConfig, StudioFilesError> {
    let supplied = file.ok_or(StudioFilesError::FileDataRequired)?;
    let ExtractedFileData {
        filename,
        content,
        content_type,
        headers,
    } = extract_file_data(supplied)?;
    let length = content.len().to_string();
    let display_name = filename.unwrap_or_else(|| now_epoch.to_string());
    let body = Json::Object(IndexMap::from([(
        "file".to_owned(),
        Json::Object(IndexMap::from([(
            "display_name".to_owned(),
            Json::Text(display_name),
        )])),
    )]));
    let data =
        serde_json::to_string(&body).map_err(|source| StudioFilesError::Encode { source })?;
    Ok(TwoStepFileUploadConfig {
        initial_request: TwoStepFileUploadRequest {
            method: "POST".to_owned(),
            url: upload_url.to_owned(),
            headers: start_headers(content_type, &headers, length.clone()),
            data: UploadData::Text(data),
        },
        upload_request: TwoStepFileUploadRequest {
            method: "POST".to_owned(),
            url: String::new(),
            headers: final_headers(length),
            data: UploadData::Bytes(content),
        },
        upload_url_location: UploadUrlLocation::Headers,
        upload_url_key: Some(UPLOAD_URL_HEADER.to_owned()),
    })
}

fn start_headers(
    content_type: String,
    custom: &BTreeMap<String, String>,
    length: String,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("X-Goog-Upload-Command".to_owned(), "start".to_owned()),
        ("X-Goog-Upload-Header-Content-Length".to_owned(), length),
        ("X-Goog-Upload-Header-Content-Type".to_owned(), content_type),
        ("X-Goog-Upload-Protocol".to_owned(), "resumable".to_owned()),
    ]);
    // Extracted custom headers win over the resumable-upload headers, as the source's dict update does.
    headers.extend(
        custom
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    headers
}

fn final_headers(length: String) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("Content-Length".to_owned(), length),
        (
            "X-Goog-Upload-Command".to_owned(),
            "upload, finalize".to_owned(),
        ),
        ("X-Goog-Upload-Offset".to_owned(), "0".to_owned()),
    ])
}
