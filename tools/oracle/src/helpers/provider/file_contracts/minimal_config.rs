use std::sync::{Arc, Mutex};

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::records::{
    BinaryContent, FileContentRequest, FileCreate, FileDeleted, FileObject,
};
use betterlite_provider::api::file_contracts::base::BaseConfig;
use betterlite_provider::api::file_contracts::config::{
    BaseFilesConfig, CompleteUrlRequest, CreateFileBody, FileLogging, FileOpenAiParam, FileParams,
    FileResponse, Headers, PreparedRequest,
};
use betterlite_provider::api::file_contracts::error::{BaseLlmException, FileContractError};
use secrecy::ExposeSecret as _;
use serde::Serialize;

#[derive(Default)]
pub(super) struct Config {
    pub(super) complete_url: CompleteUrl,
}

#[derive(Default)]
pub(super) enum CompleteUrl {
    #[default]
    Default,
    Recording(Arc<Mutex<Vec<RecordedCall>>>),
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RecordedCall {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) model: String,
    pub(super) optional_params: FileParams,
    pub(super) litellm_params: FileParams,
}

struct BaseDefaults;

impl BaseConfig for BaseDefaults {}

impl BaseConfig for Config {
    fn get_complete_url(
        &self,
        request: CompleteUrlRequest<'_>,
    ) -> Result<String, FileContractError> {
        match &self.complete_url {
            CompleteUrl::Default => BaseDefaults.get_complete_url(request),
            CompleteUrl::Recording(calls) => {
                let call = RecordedCall {
                    api_base: request.api_base.map(str::to_owned),
                    api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
                    model: request.model.to_owned(),
                    optional_params: request.optional_params.clone(),
                    litellm_params: request.litellm_params.clone(),
                };
                let Ok(mut recorded) = calls.lock() else {
                    return Err(FileContractError::Body(String::from("poisoned recording")));
                };
                recorded.push(call);
                Ok(String::from("https://mock.invalid/v1/files"))
            }
        }
    }
}

type Prepared = Result<PreparedRequest, FileContractError>;
type Reply<T> = Result<T, FileContractError>;

fn unsupported() -> FileContractError {
    FileContractError::Upstream(BaseLlmException {
        status_code: 501,
        message: String::from("not implemented"),
        headers: Headers::new(),
    })
}

impl BaseFilesConfig for Config {
    fn custom_llm_provider(&self) -> ProviderId {
        ProviderId::Openai
    }

    fn get_supported_openai_params(&self, _model: &str) -> Vec<FileOpenAiParam> {
        Vec::new()
    }

    fn transform_create_file_request(
        &self,
        _model: &str,
        _create_file_data: &FileCreate,
        _optional_params: &FileParams,
        _litellm_params: &FileParams,
    ) -> Result<CreateFileBody, FileContractError> {
        Err(unsupported())
    }

    fn transform_create_file_response(
        &self,
        _model: Option<&str>,
        _raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        _litellm_params: &FileParams,
    ) -> Reply<FileObject> {
        Err(unsupported())
    }

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_request(
        &self,
        _file_id: &str,
        _optional_params: &FileParams,
        _litellm_params: &FileParams,
    ) -> Prepared {
        Err(unsupported())
    }

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_response(
        &self,
        _raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        _litellm_params: &FileParams,
    ) -> Reply<FileObject> {
        Err(unsupported())
    }

    fn transform_delete_file_request(
        &self,
        _file_id: &str,
        _optional_params: &FileParams,
        _litellm_params: &FileParams,
    ) -> Prepared {
        Err(unsupported())
    }

    fn transform_delete_file_response(
        &self,
        _raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        _litellm_params: &FileParams,
    ) -> Reply<FileDeleted> {
        Err(unsupported())
    }

    fn transform_list_files_request(
        &self,
        _purpose: Option<&str>,
        _optional_params: &FileParams,
        _litellm_params: &FileParams,
    ) -> Prepared {
        Err(unsupported())
    }

    fn transform_list_files_response(
        &self,
        _raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        _litellm_params: &FileParams,
    ) -> Reply<Vec<FileObject>> {
        Err(unsupported())
    }

    fn transform_file_content_request(
        &self,
        _file_content_request: &FileContentRequest,
        _optional_params: &FileParams,
        _litellm_params: &FileParams,
    ) -> Prepared {
        Err(unsupported())
    }

    // ast-grep-ignore: no-long-names
    fn transform_file_content_response(
        &self,
        _raw_response: &FileResponse,
        _logging_obj: &mut FileLogging,
        _litellm_params: &FileParams,
    ) -> Reply<BinaryContent> {
        Err(unsupported())
    }
}
