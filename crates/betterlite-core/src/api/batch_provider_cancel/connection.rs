use betterlite_config::sections::batch_provider_cancel::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, request_timeout,
    resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::CancelBatchCall;

// the kwargs cancel_batch reads through GenericLiteLLMParams and get_litellm_params
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CancelParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub aws_region_name: Option<String>,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub timeout: Option<CancelTimeout>,
    pub request_timeout: Option<f64>,
    pub max_retries: Option<i64>,
}

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CancelTimeout {
    Plain(TimeoutValue),
    Structured(StructuredTimeout),
}

impl From<&Settings> for ResourceSdkSettings {
    fn from(settings: &Settings) -> Self {
        Self {
            api_base: settings.api_base.clone(),
            api_version: settings.api_version.clone(),
            api_key: settings.api_key.clone(),
            openai_key: settings.openai_key.clone(),
            azure_key: settings.azure_key.clone(),
            azure_ad_token: settings.azure_ad_token.clone(),
            organization: settings.organization.clone(),
            vertex_project: settings.vertex_project.clone(),
            vertex_location: settings.vertex_location.clone(),
            openai_base_url: settings.openai_base_url.clone(),
            openai_api_base: settings.openai_api_base.clone(),
            openai_api_key: settings.openai_api_key.clone(),
            openai_organization: settings.openai_organization.clone(),
            azure_api_base: settings.azure_api_base.clone(),
            azure_api_version: settings.azure_api_version.clone(),
            azure_openai_api_key: settings.azure_openai_api_key.clone(),
            azure_api_key: settings.azure_api_key.clone(),
            vertexai_project: settings.vertexai_project.clone(),
            vertexai_location: settings.vertexai_location.clone(),
            vertexai_credentials: settings.vertexai_credentials.clone(),
        }
    }
}

impl From<&CancelParams> for ResourceSdkRequestOptions {
    fn from(params: &CancelParams) -> Self {
        Self {
            api_base: params.api_base.clone(),
            api_version: params.api_version.clone(),
            api_key: params.api_key.clone(),
            organization: params.organization.clone(),
            vertex_project: params.vertex_project.clone(),
            vertex_location: params.vertex_location.clone(),
            vertex_credentials: params.vertex_credentials.clone(),
        }
    }
}

// cancel_batch's only fallback the shared settings lack: the AD token get_azure_ad_token reads
#[must_use]
pub fn azure_ad_token<'settings>(
    params: &'settings CancelParams,
    settings: &'settings ResourceSdkSettings,
) -> Option<&'settings SecretString> {
    [
        params.azure_ad_token.as_ref(),
        settings.azure_ad_token.as_ref(),
    ]
    .into_iter()
    .flatten()
    .find(|token| !token.expose_secret().is_empty())
}

pub fn resolve_timeout(call: &CancelBatchCall) -> Result<FineTuningTimeout, TimeoutError> {
    let provider = call.provider.0.as_str();
    match configured(call.params.timeout.as_ref()) {
        Configured::Structured(timeout) => {
            resolve_fine_tuning_timeout(None, provider, Some(timeout))
        }
        Configured::Plain(value) => resolve_fine_tuning_timeout(Some(value), provider, None),
        // optional_params.timeout or request_timeout or 600, where a zero or an empty value falls through
        Configured::Unset => Ok(FineTuningTimeout::Seconds(request_timeout(
            None,
            call.params.request_timeout,
        )?)),
    }
}

enum Configured<'value> {
    Unset,
    Plain(&'value TimeoutValue),
    Structured(StructuredTimeout),
}

fn configured(timeout: Option<&CancelTimeout>) -> Configured<'_> {
    match timeout {
        Some(CancelTimeout::Structured(structured)) => Configured::Structured(*structured),
        Some(CancelTimeout::Plain(value @ TimeoutValue::Number(seconds)))
            if *seconds != 0.0_f64 =>
        {
            Configured::Plain(value)
        }
        Some(CancelTimeout::Plain(value @ TimeoutValue::String(text))) if !text.is_empty() => {
            Configured::Plain(value)
        }
        None | Some(CancelTimeout::Plain(_)) => Configured::Unset,
    }
}
