use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::sdk::TimeoutPhases;
use betterlite_openai::fine_tuning::client::JobsApi;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use crate::api::batch_provider_cancel::CancelErrorFields;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, request_timeout,
    resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

mod tests;

const OPENAI_PROVIDER: &str = "openai";

#[expect(
    clippy::unnecessary_wraps,
    reason = "serde's default hook must return the field's Option type"
)]
fn default_provider() -> Option<String> {
    Some(OPENAI_PROVIDER.to_owned())
}

// GenericLiteLLMParams.timeout: seconds, a number written as text, or the pieces of an httpx.Timeout
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OptionalTimeout {
    Seconds(f64),
    Text(String),
    Phases(TimeoutPhases),
}

// the kwargs cancel_fine_tuning_job reads through GenericLiteLLMParams and the raw kwargs dict
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FineTuningOptionalParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: Option<OptionalTimeout>,
    pub request_timeout: Option<f64>,
    pub max_retries: Option<i64>,
    // GenericLiteLLMParams accepts it, and cancel_fine_tuning_job never reads it on either branch
    pub azure_ad_token: Option<SecretString>,
}

// cancel_fine_tuning_job's arguments; acancel_fine_tuning_job leaves its flag in the kwargs dict
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineTuningCancelCall {
    pub fine_tuning_job_id: String,
    #[serde(default = "default_provider")]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub params: FineTuningOptionalParams,
    // acancel_fine_tuning_job sets this; cancel_fine_tuning_job reads it with `is True`
    #[serde(default)]
    pub acancel_fine_tuning_job: Option<Json>,
    // the in-process SDK client a caller may supply, which only the openai branch receives
    #[serde(skip)]
    pub client: Option<Box<dyn JobsApi + Send + Sync>>,
}

// the request-level values the provider fallback chains read first
impl From<&FineTuningOptionalParams> for ResourceSdkRequestOptions {
    fn from(params: &FineTuningOptionalParams) -> Self {
        Self {
            api_base: params.api_base.clone(),
            api_version: params.api_version.clone(),
            api_key: params.api_key.clone(),
            organization: params.organization.clone(),
            vertex_project: None,
            vertex_location: None,
            vertex_credentials: None,
        }
    }
}

// the values every provider branch reads after the shared timeout resolution
#[derive(Debug, Clone, Copy)]
pub struct FineTuningCancelValues<'call> {
    pub is_async: bool,
    pub job_id: &'call str,
    pub timeout: &'call FineTuningTimeout,
    pub max_retries: Option<i64>,
}

pub struct OpenAiFineTuningCancel<'call> {
    pub values: FineTuningCancelValues<'call>,
    // the openai branch always has a base: the call's, the settings' or the default
    pub api_base: String,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub organization: Option<&'call str>,
    pub client: Option<&'call (dyn JobsApi + Send + Sync)>,
}

#[derive(Debug, Clone, Copy)]
pub struct AzureFineTuningCancel<'call> {
    pub values: FineTuningCancelValues<'call>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub organization: Option<&'call str>,
}

// cancel_fine_tuning_job's two branches; every other provider answers 400
pub enum FineTuningCancelTarget<'call> {
    OpenAi(OpenAiFineTuningCancel<'call>),
    Azure(AzureFineTuningCancel<'call>),
}

#[derive(Debug, Error)]
pub enum FineTuningCancelError<DispatchError = Infallible>
where
    DispatchError: Error + 'static,
{
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    // the source message names the wrong operation on purpose: LiteLLM copies its create_batch text
    #[error("LiteLLM doesn't support {provider} for 'create_batch'. Only 'openai' is supported.")]
    Unsupported { provider: String },
    #[error(transparent)]
    Dispatch(DispatchError),
}

impl<DispatchError> FineTuningCancelError<DispatchError>
where
    DispatchError: Error + 'static,
{
    pub const fn error_fields(&self) -> Option<CancelErrorFields> {
        match self {
            Self::Unsupported { .. } => Some(CancelErrorFields {
                status: 400,
                error_type: None,
                param: None,
                code: None,
            }),
            Self::Timeout(_) | Self::Dispatch(_) => None,
        }
    }
}

pub trait FineTuningCancelDispatch: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn cancel(
        &self,
        target: FineTuningCancelTarget<'_>,
    ) -> impl Future<Output = Result<FineTuneJob, Self::Error>> + Send;
}

pub struct FineTuningCancel<Dispatch> {
    pub dispatch: Dispatch,
    pub settings: Settings,
}

// what a run resolves before the provider branches see the call
struct Prepared {
    is_async: bool,
    timeout: FineTuningTimeout,
    settings: ResourceSdkSettings,
    options: ResourceSdkRequestOptions,
}

impl<Dispatch> FineTuningCancel<Dispatch>
where
    Dispatch: FineTuningCancelDispatch,
    Dispatch::Error: Error + Send + Sync + 'static,
{
    pub async fn run(
        &self,
        call: &FineTuningCancelCall,
    ) -> Result<FineTuneJob, FineTuningCancelError<Dispatch::Error>> {
        let provider = call.custom_llm_provider.as_deref();
        let prepared = Prepared {
            // acancel_fine_tuning_job sets the flag, and only a literal true reaches the client
            is_async: matches!(
                call.acancel_fine_tuning_job.as_ref(),
                Some(Json::Bool(true))
            ),
            timeout: resolve_timeout(call, provider)?,
            settings: ResourceSdkSettings::from(&self.settings),
            options: ResourceSdkRequestOptions::from(&call.params),
        };
        let Some(target) = cancel_target(call, &prepared) else {
            return Err(FineTuningCancelError::Unsupported {
                provider: provider.unwrap_or("None").to_owned(),
            });
        };
        self.dispatch
            .cancel(target)
            .await
            .map_err(FineTuningCancelError::Dispatch)
    }
}

// optional_params.timeout or request_timeout or 600, and only openai, azure and bedrock keep the phases
fn resolve_timeout(
    call: &FineTuningCancelCall,
    provider: Option<&str>,
) -> Result<FineTuningTimeout, TimeoutError> {
    let provider_name = provider.unwrap_or_default();
    match call.params.timeout.as_ref() {
        Some(OptionalTimeout::Phases(phases)) => {
            resolve_fine_tuning_timeout(None, provider_name, Some(structured_timeout(phases)))
        }
        Some(OptionalTimeout::Seconds(seconds)) if *seconds != 0.0_f64 => {
            resolve_fine_tuning_timeout(Some(&TimeoutValue::Number(*seconds)), provider_name, None)
        }
        Some(OptionalTimeout::Text(text)) if !text.is_empty() => resolve_fine_tuning_timeout(
            Some(&TimeoutValue::String(text.clone())),
            provider_name,
            None,
        ),
        None | Some(OptionalTimeout::Seconds(_) | OptionalTimeout::Text(_)) => Ok(
            FineTuningTimeout::Seconds(request_timeout(None, call.params.request_timeout)?),
        ),
    }
}

const fn structured_timeout(phases: &TimeoutPhases) -> StructuredTimeout {
    StructuredTimeout {
        connect: phases.connect,
        read: phases.read,
        write: phases.write,
        pool: phases.pool,
    }
}

fn cancel_values<'call>(
    call: &'call FineTuningCancelCall,
    prepared: &'call Prepared,
) -> FineTuningCancelValues<'call> {
    FineTuningCancelValues {
        is_async: prepared.is_async,
        job_id: &call.fine_tuning_job_id,
        timeout: &prepared.timeout,
        max_retries: call.params.max_retries,
    }
}

fn cancel_target<'call>(
    call: &'call FineTuningCancelCall,
    prepared: &'call Prepared,
) -> Option<FineTuningCancelTarget<'call>> {
    let values = cancel_values(call, prepared);
    match call.custom_llm_provider.as_deref() {
        Some(OPENAI_PROVIDER) => Some(FineTuningCancelTarget::OpenAi(OpenAiFineTuningCancel {
            values,
            api_base: prepared.settings.openai_base(&prepared.options),
            api_key: prepared.settings.openai_api_key(&prepared.options),
            api_version: prepared.settings.openai_api_version(&prepared.options),
            organization: prepared.settings.openai_organization(&prepared.options),
            client: call.client.as_deref(),
        })),
        // the azure kwargs extra_body is reassigned and its azure_ad_token dropped, so nothing carries it here
        Some("azure") => Some(FineTuningCancelTarget::Azure(AzureFineTuningCancel {
            values,
            api_base: prepared.settings.azure_base(&prepared.options),
            api_key: prepared.settings.azure_api_key(&prepared.options),
            api_version: prepared.settings.azure_version(&prepared.options),
            organization: prepared.settings.azure_organization(&prepared.options),
        })),
        _ => None,
    }
}
