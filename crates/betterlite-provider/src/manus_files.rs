use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_openai::files::records::{
    BinaryContent, FileContentRequest, FileCreate, FileDeleted, FileObject,
};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::SecretString;

use crate::api::file_contracts::base::BaseConfig;
use crate::api::file_contracts::config::{
    BaseFilesConfig, CompleteUrlRequest, CreateFileBody, FileLogging, FileOpenAiParam, FileParams,
    FileResponse, Headers,
};
use crate::api::file_contracts::error::{BaseLlmException, FileContractError};

pub mod create;
mod credentials;
pub mod error;
mod params;
pub mod reply;
mod url;

mod tests;

// LiteLLM defaults the base when neither the deployment nor the SDK sets one.
pub const MANUS_API_BASE: &str = "https://api.manus.im";
pub const MANUS_API_BASE_ENV: &str = "MANUS_API_BASE";
pub const MANUS_API_KEY_ENV: &str = "MANUS_API_KEY";
// Manus spells the auth header API_KEY, not Authorization: Bearer.
pub const API_KEY_HEADER: &str = "API_KEY";
pub const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub const JSON_CONTENT_TYPE: &str = "application/json";
pub const FILE_ID_FIELD: &str = "file_id";
pub const UPLOAD_URL_KEY: &str = "upload_url";
pub const DEFAULT_FILENAME_PREFIX: &str = "file_";

// Every outside value the config reads is passed in when it is built, so it never touches the process environment.
pub struct ManusFilesConfig {
    pub globals: GlobalCredentials,
    pub secrets: SecretLookup,
    pub environment: BTreeMap<String, String>,
    pub clock: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl ManusFilesConfig {
    pub fn validate_environment(
        &self,
        headers: &mut Headers,
        api_key: Option<&SecretString>,
    ) -> Result<(), error::ManusFilesError> {
        credentials::validate_environment(self, headers, api_key)?;
        Ok(())
    }

    // LiteLLM hands the upstream status, message and headers to the OpenAI error class unchanged.
    #[must_use]
    pub fn get_error_class(message: &str, status_code: u16, headers: Headers) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
        }
    }

    fn file_url_from_params(
        &self,
        file_id: &str,
        litellm_params: &FileParams,
    ) -> Result<String, error::ManusFilesError> {
        let base = url::litellm_files_url(
            litellm_params,
            &self.globals,
            &self.secrets,
            &self.environment,
        )?;
        let encoded = url::encoded_file_id(file_id)?;
        Ok(format!("{base}/{encoded}"))
    }
}

impl BaseConfig for ManusFilesConfig {
    fn get_complete_url(
        &self,
        request: CompleteUrlRequest<'_>,
    ) -> Result<String, FileContractError> {
        let url = url::files_url(
            request.api_base,
            &self.globals,
            &self.secrets,
            &self.environment,
        )?;
        Ok(url)
    }
}

impl BaseFilesConfig for ManusFilesConfig {
    fn custom_llm_provider(&self) -> ProviderId {
        ProviderId::Manus
    }

    fn get_supported_openai_params(&self, _model: &str) -> Vec<FileOpenAiParam> {
        vec![FileOpenAiParam::Purpose]
    }

    fn transform_create_file_request(
        &self,
        _model: &str,
        create_file_data: &FileCreate,
        _optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<CreateFileBody, FileContractError> {
        let plan = create::upload_plan(self, Some(&create_file_data.file), litellm_params)?;
        Ok(CreateFileBody::TwoStep(Box::new(plan)))
    }

    fn transform_create_file_response(
        &self,
        _model: Option<&str>,
        raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        litellm_params: &FileParams,
    ) -> Result<FileObject, FileContractError> {
        let file = reply::create_file_object(raw_response, litellm_params, (self.clock)())?;
        Ok(file)
    }

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_request(
        &self,
        file_id: &str,
        _optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<(String, FileParams), FileContractError> {
        let url = self.file_url_from_params(file_id, litellm_params)?;
        Ok((url, FileParams::new()))
    }

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_response(
        &self,
        raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        litellm_params: &FileParams,
    ) -> Result<FileObject, FileContractError> {
        let file = reply::create_file_object(raw_response, litellm_params, (self.clock)())?;
        Ok(file)
    }

    fn transform_delete_file_request(
        &self,
        file_id: &str,
        _optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<(String, FileParams), FileContractError> {
        let url = self.file_url_from_params(file_id, litellm_params)?;
        Ok((url, FileParams::new()))
    }

    fn transform_delete_file_response(
        &self,
        raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        _litellm_params: &FileParams,
    ) -> Result<FileDeleted, FileContractError> {
        let deleted = reply::decode_deleted_file(&raw_response.body)?;
        Ok(deleted)
    }

    fn transform_list_files_request(
        &self,
        purpose: Option<&str>,
        _optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<(String, FileParams), FileContractError> {
        let base = url::litellm_files_url(
            litellm_params,
            &self.globals,
            &self.secrets,
            &self.environment,
        )?;
        let mut params = FileParams::new();
        if let Some(sent_purpose) = purpose.filter(|value| !value.is_empty()) {
            params.insert(String::from("purpose"), serde_json::json!(sent_purpose));
        }
        Ok((base, params))
    }

    fn transform_list_files_response(
        &self,
        raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        _litellm_params: &FileParams,
    ) -> Result<Vec<FileObject>, FileContractError> {
        let files = reply::decode_file_list(&raw_response.body, (self.clock)())?;
        Ok(files)
    }

    fn transform_file_content_request(
        &self,
        file_content_request: &FileContentRequest,
        _optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<(String, FileParams), FileContractError> {
        let base = url::litellm_files_url(
            litellm_params,
            &self.globals,
            &self.secrets,
            &self.environment,
        )?;
        let encoded = url::encoded_file_id(&file_content_request.file_id)?;
        Ok((format!("{base}/{encoded}/content"), FileParams::new()))
    }

    // ast-grep-ignore: no-long-names
    fn transform_file_content_response(
        &self,
        raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        _litellm_params: &FileParams,
    ) -> Result<BinaryContent, FileContractError> {
        let headers = raw_response
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        Ok(BinaryContent {
            status: raw_response.status_code,
            headers,
            body: raw_response.body.clone(),
        })
    }
}
