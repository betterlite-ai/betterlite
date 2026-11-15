use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::batch_provider_list::{Raw, RawSettings};
use betterlite_core::api::batch_provider_list::call::{
    AzureBatchListParams, BatchListProviderCall,
};
use betterlite_core::api::batch_provider_list::error::BatchProviderListError;
use betterlite_core::api::batch_provider_list::{
    BatchListTimeoutValue, BatchProviderList, BatchProviderListInput, BatchProviderListResult,
};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_core::genai_setup::params::VertexCredentials;
use betterlite_openai::batches::sdk::{Timeout, TimeoutPhases};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    request: BatchProviderListInput,
    #[serde(default)]
    litellm_settings: RawSettings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

fn secret_value(value: Option<&SecretString>) -> Value {
    value.map_or(Value::Null, |text| {
        Value::String(text.expose_secret().to_owned())
    })
}

fn phases_value(phases: &TimeoutPhases) -> Value {
    json!({
        "connect": phases.connect,
        "read": phases.read,
        "write": phases.write,
        "pool": phases.pool,
    })
}

fn timeout_value(timeout: &Timeout) -> Value {
    match timeout {
        Timeout::Seconds(seconds) => json!(seconds),
        Timeout::Phases(phases) => phases_value(phases),
    }
}

fn raw_timeout_value(value: Option<&BatchListTimeoutValue>) -> Value {
    match value {
        None => Value::Null,
        Some(BatchListTimeoutValue::Seconds(seconds)) => json!(seconds),
        Some(BatchListTimeoutValue::Text(text)) => json!(text),
        Some(BatchListTimeoutValue::Phases(phases)) => phases_value(phases),
    }
}

fn params_value(params: &AzureBatchListParams) -> Value {
    json!({
        "api_key": secret_value(params.api_key.as_ref()),
        "api_base": params.api_base,
        "api_version": params.api_version,
        "max_retries": params.max_retries,
        "timeout": raw_timeout_value(params.timeout.as_ref()),
        "azure_ad_token": secret_value(params.azure_ad_token.as_ref()),
        "tenant_id": params.tenant_id,
        "client_id": params.client_id,
        "client_secret": secret_value(params.client_secret.as_ref()),
        "azure_scope": params.azure_scope,
        "azure_username": params.azure_username,
        "azure_password": secret_value(params.azure_password.as_ref()),
    })
}

fn credentials_value(credentials: Option<&VertexCredentials>) -> HelperResult {
    match credentials {
        None => Ok(Value::Null),
        Some(VertexCredentials::Path(path)) => Ok(Value::String(path.clone())),
        Some(VertexCredentials::ServiceAccount(account)) => Ok(serde_json::to_value(account)?),
    }
}

fn call_value(result: BatchProviderListResult) -> HelperResult {
    let is_async = result.is_async;
    let window = result.call.window;
    // every OpenAI-compatible provider reaches LiteLLM's openai client, so the record says openai
    let value = match result.call.provider {
        BatchListProviderCall::OpenAiCompatible(call) => json!({
            "provider": "openai", "is_async": is_async, "after": window.after,
            "limit": window.limit, "api_base": call.api_base,
            "api_key": secret_value(call.api_key.as_ref()),
            "organization": call.organization, "timeout": timeout_value(&window.timeout),
            "max_retries": call.max_retries,
        }),
        BatchListProviderCall::Azure(call) => json!({
            "provider": "azure", "is_async": is_async, "api_base": call.api_base,
            "api_key": secret_value(call.api_key.as_ref()), "api_version": call.api_version,
            "timeout": timeout_value(&window.timeout), "max_retries": call.max_retries,
            "litellm_params": params_value(&call.litellm_params),
        }),
        BatchListProviderCall::VertexAi(call) => {
            let credentials = credentials_value(call.vertex_credentials.as_ref())?;
            json!({
                "provider": "vertex_ai", "is_async": is_async, "after": window.after,
                "limit": window.limit, "api_base": call.api_base,
                "vertex_project": call.vertex_project, "vertex_location": call.vertex_location,
                "vertex_credentials": credentials,
                "timeout": timeout_value(&window.timeout), "max_retries": call.max_retries,
            })
        }
        BatchListProviderCall::Xai(call) => json!({
            "provider": "xai", "is_async": is_async, "after": window.after,
            "limit": window.limit, "api_base": call.api_base,
            "api_key": secret_value(call.api_key.as_ref()),
            "timeout": timeout_value(&window.timeout),
        }),
    };
    Ok(value)
}

fn failure(error: &BatchProviderListError) -> Box<HelperError> {
    let (class, status): (&str, Option<u16>) = match error {
        BatchProviderListError::UnsupportedProvider { .. } => ("BadRequestError", Some(400)),
        BatchProviderListError::InvalidTimeout { .. }
        | BatchProviderListError::InvalidVertexCredentials
        | BatchProviderListError::NonFiniteTimeout => ("ValueError", None),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let raw = Raw {
            litellm_settings: input.litellm_settings.clone(),
        };
        let settings = raw.convert(&input.environment).or_config_error()?;
        let service = BatchProviderList {
            settings: ResourceSdkSettings::from(settings),
        };
        let result = service
            .run(&input.request)
            .map_err(|error| failure(&error))?;
        call_value(result)
    }
}
