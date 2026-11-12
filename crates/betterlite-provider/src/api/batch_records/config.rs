use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::batches::records::BatchObject;
use secrecy::SecretString;

use super::BatchBody;
use super::error::BatchContractError;
use crate::api::file_contracts::config::{FileLogging, FileParams, FileResponse, Headers};
use crate::api::file_contracts::error::BaseLlmException;

pub struct BatchEnvironment<'request> {
    pub headers: &'request Headers,
    pub model: &'request str,
    pub messages: &'request [Json],
    pub optional_params: &'request FileParams,
    pub litellm_params: &'request FileParams,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
}

pub struct BatchUrlRequest<'request> {
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub model: &'request str,
    pub optional_params: &'request FileParams,
    pub litellm_params: &'request FileParams,
    pub data: &'request CreateBatchRequest,
}

// BaseBatchesConfig: every provider answers all of it, since LiteLLM's methods are all abstract
pub trait BaseBatchesConfig: Send + Sync {
    fn custom_llm_provider(&self) -> ProviderId;

    // the class-level settings the provider declares, with the unset ones left out
    fn get_config(&self) -> FileParams;

    fn validate_environment(
        &self,
        request: BatchEnvironment<'_>,
    ) -> Result<Headers, BatchContractError>;

    fn get_complete_batch_url(
        &self,
        request: BatchUrlRequest<'_>,
    ) -> Result<String, BatchContractError>;

    fn transform_create_batch_request(
        &self,
        model: &str,
        create_batch_data: &CreateBatchRequest,
        optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<BatchBody, BatchContractError>;

    // ast-grep-ignore: no-long-names
    fn transform_create_batch_response(
        &self,
        model: Option<&str>,
        raw_response: &FileResponse,
        logging_obj: &mut FileLogging,
        litellm_params: &FileParams,
    ) -> Result<BatchObject, BatchContractError>;

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_batch_request(
        &self,
        batch_id: &str,
        optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<BatchBody, BatchContractError>;

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_batch_response(
        &self,
        model: Option<&str>,
        raw_response: &FileResponse,
        logging_obj: &mut FileLogging,
        litellm_params: &FileParams,
    ) -> Result<BatchObject, BatchContractError>;

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> BaseLlmException;
}
