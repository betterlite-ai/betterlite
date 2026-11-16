pub mod error;
pub mod execution;
mod mock;
mod prepare;

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::ContainerFileObject;
use betterlite_openai::containers::container_file_http::ContainerFileUpload;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::error::{ContainerFileUploadError, ContainerFileUploadFailure};
use self::execution::{
    AsyncUploadExecutor, ContainerFileUploadCall, UploadExecutor, UploadFailure,
};
use self::prepare::{Prepared, prepare_upload};

pub(crate) const DEFAULT_PROVIDER: &str = "openai";
// LiteLLM's declared default for this function's timeout argument, before `timeout or DEFAULT_REQUEST_TIMEOUT`
pub(crate) const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;
pub(crate) const ENDPOINT_NAME: &str = "upload_container_file";
pub(crate) const CALL_TYPE: &str = "upload_container_file";

// the named arguments upload_container_file takes, with the values @client injects split out
pub struct ContainerFileUploadRequest<'request> {
    pub container_id: &'request str,
    pub file: &'request ContainerFileUpload,
    // the declared default is 600 seconds; an explicit zero is LiteLLM's `timeout or DEFAULT_REQUEST_TIMEOUT`
    pub timeout: Option<f64>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<&'request str>,
    pub api_version: Option<&'request str>,
    // the declared default is "openai"
    pub custom_llm_provider: Option<&'request str>,
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    pub extra_query: Option<&'request IndexMap<String, ParamValue>>,
    // accepted to match the signature; LiteLLM never forwards it
    pub extra_body: Option<&'request IndexMap<String, JsonValue>>,
    pub kwargs: &'request IndexMap<String, Json>,
}

// the collaborators the operation needs: B247's logging object, B185's redactor and REQUEST_TIMEOUT's seconds
pub struct ContainerFileUploadDeps<'deps> {
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    // LiteLLM's request_timeout constant that REQUEST_TIMEOUT overrides, used only where LiteLLM reaches it
    pub request_timeout_seconds: f64,
}

// LiteLLM's upload_container_file: the mock, the named credentials, the managed id, the logging update, then the handler
pub fn upload_container_file<Executor>(
    executor: &Executor,
    deps: ContainerFileUploadDeps<'_>,
    request: &ContainerFileUploadRequest<'_>,
) -> Result<ContainerFileObject, ContainerFileUploadFailure<Executor::Failure>>
where
    Executor: UploadExecutor,
{
    let prepared = prepare_upload::<Executor::Failure>(&deps, request)?;
    let prepared_upload = match prepared {
        Prepared::Mock(object) => return Ok(object),
        Prepared::Call(upload) => upload,
    };
    let call = ContainerFileUploadCall {
        endpoint_name: ENDPOINT_NAME,
        provider_config: prepared_upload.provider_config,
        litellm_params: &prepared_upload.litellm_params,
        logging: deps.logging,
        extra_headers: request.extra_headers,
        extra_query: request.extra_query,
        timeout_seconds: prepared_upload.timeout_seconds,
        container_id: &prepared_upload.container_id,
        file: request.file,
    };
    finish(
        executor.upload(&call),
        &prepared_upload.provider,
        deps.redactor,
    )
}

// aupload_container_file: the same operation, awaited through the async executor the same handler offers
pub async fn aupload_container_file<Executor>(
    executor: &Executor,
    deps: ContainerFileUploadDeps<'_>,
    request: &ContainerFileUploadRequest<'_>,
) -> Result<ContainerFileObject, ContainerFileUploadFailure<Executor::Failure>>
where
    Executor: AsyncUploadExecutor,
{
    let prepared = prepare_upload::<Executor::Failure>(&deps, request)?;
    let prepared_upload = match prepared {
        Prepared::Mock(object) => return Ok(object),
        Prepared::Call(upload) => upload,
    };
    let call = ContainerFileUploadCall {
        endpoint_name: ENDPOINT_NAME,
        provider_config: prepared_upload.provider_config,
        litellm_params: &prepared_upload.litellm_params,
        logging: deps.logging,
        extra_headers: request.extra_headers,
        extra_query: request.extra_query,
        timeout_seconds: prepared_upload.timeout_seconds,
        container_id: &prepared_upload.container_id,
        file: request.file,
    };
    finish(
        executor.aupload(&call).await,
        &prepared_upload.provider,
        deps.redactor,
    )
}

// exception_type(model="", custom_llm_provider=..., original_exception=...)
fn finish<Failure>(
    sent: Result<ContainerFileObject, Failure>,
    provider: &str,
    redactor: &Redactor,
) -> Result<ContainerFileObject, ContainerFileUploadFailure<Failure>>
where
    Failure: UploadFailure,
{
    sent.map_err(|carried| {
        error::failure(
            ContainerFileUploadError::Provider(carried),
            provider,
            redactor,
        )
    })
}

#[path = "container_file_upload/tests/support.rs"]
mod test_support;

#[path = "container_file_upload/tests/upload_calls.rs"]
mod upload_calls;

#[path = "container_file_upload/tests/mock_calls.rs"]
mod mock_calls;

#[path = "container_file_upload/tests/logging_calls.rs"]
mod logging_calls;

#[path = "container_file_upload/tests/failure_calls.rs"]
mod failure_calls;

#[path = "container_file_upload/tests/async_calls.rs"]
mod async_calls;
