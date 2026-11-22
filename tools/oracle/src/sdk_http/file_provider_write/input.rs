use betterlite_core::api::file_http_upload::service::Mode;
use betterlite_core::api::file_provider_write::service::FileWrite;
use betterlite_core::api::file_provider_write::{
    DeleteInput, UploadInput, WriteParams, WriteTimeout,
};
use betterlite_core::api::fine_tuning_read::ReadTimeout;
use betterlite_core::api::resource_sdk_settings::timeout::{
    StructuredTimeout, TimeoutError, TimeoutValue,
};
use betterlite_openai::files::records::{CreatePurpose, ExpiresAfter, FileUpload};
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::outcome::step_outcome;
use super::providers::OracleProviders;
use super::{Params, Purpose, Step, StepOutcome, UploadStep};

pub(super) async fn run_step(
    write: &FileWrite<'_, OracleProviders>,
    step: &Step,
    api_base: &str,
) -> Result<StepOutcome, SdkHttpError> {
    let mode = match step {
        Step::CreateFile(_) | Step::FileDelete(_) => Mode::Sync,
        Step::ACreateFile(_) | Step::AFileDelete(_) => Mode::Asynchronous,
    };
    match step {
        Step::CreateFile(upload) | Step::ACreateFile(upload) => {
            let input = upload_input(upload, api_base)?;
            step_outcome(write.create_file(mode, &input).await)
        }
        Step::FileDelete(delete) | Step::AFileDelete(delete) => {
            let input = DeleteInput {
                file_id: delete.file_id.clone(),
                model: delete.model.clone(),
                custom_llm_provider: delete.custom_llm_provider.clone(),
                extra_headers: delete.extra_headers.clone(),
                extra_body: delete.extra_body.clone(),
                params: write_params(&delete.params, api_base)?,
            };
            step_outcome(write.delete_file(mode, &input).await)
        }
    }
}
fn upload_input(step: &UploadStep, api_base: &str) -> Result<UploadInput<(), ()>, SdkHttpError> {
    Ok(UploadInput {
        file: FileUpload {
            filename: Some(step.file.filename.clone()),
            content_type: Some(step.file.content_type.clone()),
            data: step.file.content.clone().into_bytes(),
        },
        purpose: match step.purpose {
            Purpose::Assistants => CreatePurpose::Assistants,
            Purpose::Batch => CreatePurpose::Batch,
            Purpose::FineTune => CreatePurpose::FineTune,
            Purpose::Messages => CreatePurpose::Messages,
        },
        expires_after: step.expires_after.map(|expires| ExpiresAfter {
            seconds: expires.seconds,
        }),
        custom_llm_provider: step.custom_llm_provider.clone(),
        extra_headers: step.extra_headers.clone(),
        extra_body: step.extra_body.clone(),
        params: write_params(&step.params, api_base)?,
    })
}
// the decorator always hands the call a logger; the harness base stands in for an absent api_base
fn write_params(params: &Params, api_base: &str) -> Result<WriteParams<(), ()>, SdkHttpError> {
    let timeout = params
        .timeout
        .as_ref()
        .map(|timeout| match timeout {
            ReadTimeout::Plain(value) => TimeoutValue::try_from(value).map(WriteTimeout::Value),
            ReadTimeout::Structured(phases) => {
                StructuredTimeout::try_from(phases).map(WriteTimeout::Structured)
            }
        })
        .transpose()
        .map_err(invalid_timeout)?;
    let request_timeout = params
        .request_timeout
        .as_ref()
        .map(TimeoutValue::try_from)
        .transpose()
        .map_err(invalid_timeout)?;
    Ok(WriteParams {
        api_base: Some(
            params
                .api_base
                .clone()
                .unwrap_or_else(|| api_base.to_owned()),
        ),
        api_key: params.api_key.clone().map(SecretString::from),
        api_version: params.api_version.clone(),
        organization: params.organization.clone(),
        timeout,
        request_timeout,
        max_retries: params.max_retries,
        passthrough: params.passthrough,
        litellm_call_id: None,
        id: None,
        logger: Some(()),
        client: None,
        credentials: None,
    })
}
fn invalid_timeout(error: TimeoutError) -> SdkHttpError {
    SdkHttpError::Invalid(format!("the case timeout is not a number: {error}"))
}
