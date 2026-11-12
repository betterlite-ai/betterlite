use std::collections::BTreeMap;

use betterlite_openai::api::file_type_records::{
    FileContentProvider, ProviderUploadPayload, ProviderUploadValue, TwoStepFileUploadConfig,
    TwoStepFileUploadRequest, UploadData, UploadUrlLocation,
};
use betterlite_openai::files::records::FileUpload;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::file_contracts::config::FileParams;

use super::credentials::resolve_api_key;
use super::error::ManusFilesError;
use super::params::text;
use super::url::files_url;
use super::{
    API_KEY_HEADER, CONTENT_TYPE_HEADER, DEFAULT_FILENAME_PREFIX, JSON_CONTENT_TYPE,
    ManusFilesConfig, UPLOAD_URL_KEY,
};

pub fn upload_plan(
    config: &ManusFilesConfig,
    file: Option<&FileUpload>,
    litellm_params: &FileParams,
) -> Result<TwoStepFileUploadConfig, ManusFilesError> {
    let supplied = file.ok_or(ManusFilesError::FileDataRequired)?;
    let filename = supplied
        .filename
        .clone()
        .unwrap_or_else(|| format!("{DEFAULT_FILENAME_PREFIX}{}", (config.clock)()));
    let url = files_url(
        text(litellm_params, "api_base")?,
        &config.globals,
        &config.secrets,
        &config.environment,
    )?;
    let headers = plan_headers(config, litellm_params)?;
    Ok(TwoStepFileUploadConfig {
        initial_request: TwoStepFileUploadRequest {
            method: String::from("POST"),
            url,
            headers,
            data: UploadData::Provider(ProviderUploadPayload {
                provider: FileContentProvider::Manus,
                value: ProviderUploadValue { filename },
            }),
        },
        upload_request: TwoStepFileUploadRequest {
            method: String::from("PUT"),
            url: String::new(),
            headers: BTreeMap::new(),
            data: UploadData::Bytes(supplied.data.clone()),
        },
        upload_url_location: UploadUrlLocation::Body,
        upload_url_key: Some(String::from(UPLOAD_URL_KEY)),
    })
}

fn plan_headers(
    config: &ManusFilesConfig,
    litellm_params: &FileParams,
) -> Result<BTreeMap<String, String>, ManusFilesError> {
    let explicit = text(litellm_params, "api_key")?.map(SecretString::from);
    let key = resolve_api_key(config, explicit.as_ref())?;
    Ok(BTreeMap::from([
        (String::from(API_KEY_HEADER), key.expose_secret().to_owned()),
        (
            String::from(CONTENT_TYPE_HEADER),
            String::from(JSON_CONTENT_TYPE),
        ),
    ]))
}
