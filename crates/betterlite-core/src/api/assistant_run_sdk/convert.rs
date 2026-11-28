use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::thread_runs::records::RunRequest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, TimeoutValue, request_timeout, resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::call::{RunParams, RunThreadCall, RunTimeout};
use super::error::AssistantRunError;

// the supplied client's class wins over the arun switch: a sync client stays sync even under arun_thread
pub(super) const fn client_kind(call: &RunThreadCall, arun: bool) -> ClientKind {
    match call.client.as_ref() {
        Some(client) => client.kind(),
        None if arun => ClientKind::Asynchronous,
        None => ClientKind::Sync,
    }
}

// the six keyword arguments both provider methods receive; thread_id stays the path alone
pub(super) fn run_request(call: &RunThreadCall) -> RunRequest {
    RunRequest {
        thread_id: call.thread_id.clone(),
        assistant_id: call.assistant_id.clone(),
        additional_instructions: call.additional_instructions.clone(),
        instructions: call.instructions.clone(),
        metadata: call.metadata.clone(),
        model: call.model.clone(),
        tools: call.tools.clone(),
    }
}

pub(super) fn openai_options(
    call: &RunThreadCall,
    settings: &ResourceSdkSettings,
    timeout: Duration,
) -> ClientOptions {
    let options = ResourceSdkRequestOptions::from(&call.params);
    ClientOptions {
        api_key: settings.openai_api_key(&options).cloned(),
        api_base: Some(settings.openai_base(&options)),
        timeout: Some(timeout),
        max_retries: call
            .params
            .max_retries
            .and_then(|value| u32::try_from(value).ok()),
        organization: settings.openai_organization(&options).map(str::to_owned),
    }
}

// timeout = optional_params.timeout or kwargs.get("request_timeout", 600) or 600, then float() or read
pub(super) fn facade_timeout(
    call: &RunThreadCall,
    provider: &str,
) -> Result<Duration, AssistantRunError> {
    let params = &call.params;
    let resolved = match params.timeout.as_ref() {
        Some(RunTimeout::Structured(structured)) => {
            resolve_fine_tuning_timeout(None, provider, Some(*structured))?
        }
        Some(RunTimeout::Plain(value @ TimeoutValue::Number(seconds))) if *seconds != 0.0_f64 => {
            resolve_fine_tuning_timeout(Some(value), provider, None)?
        }
        Some(RunTimeout::Plain(value @ TimeoutValue::String(text))) if !text.is_empty() => {
            resolve_fine_tuning_timeout(Some(value), provider, None)?
        }
        None | Some(RunTimeout::Plain(_)) => {
            FineTuningTimeout::Seconds(request_timeout(None, params.request_timeout)?)
        }
    };
    // B521 and B543 carry one deadline per read, so a structured timeout keeps its read component
    let seconds = match resolved {
        FineTuningTimeout::Seconds(seconds) => seconds,
        FineTuningTimeout::Structured(timeout) => timeout.read.unwrap_or(600.0_f64),
    };
    Duration::try_from_secs_f64(seconds)
        .map_err(|_error| AssistantRunError::InvalidTimeout { seconds })
}

// a supplied client's timeout is a number of seconds; httpx refuses a negative one where a Duration does
pub(super) fn supplied_timeout(
    call: &RunThreadCall,
) -> Result<Option<Duration>, AssistantRunError> {
    match call.client.as_ref().and_then(|client| client.timeout) {
        None => Ok(None),
        Some(seconds) => {
            let duration = Duration::try_from_secs_f64(seconds)
                .map_err(|_error| AssistantRunError::InvalidTimeout { seconds })?;
            Ok(Some(duration))
        }
    }
}

// litellm_params["timeout"] is the raw kwarg, which the Azure client turns into its own deadline
pub(super) fn azure_timeout(params: &RunParams) -> Result<Option<f64>, AssistantRunError> {
    let Some(timeout) = params.timeout.as_ref() else {
        return Ok(None);
    };
    match timeout {
        RunTimeout::Plain(TimeoutValue::Number(seconds)) => Ok(Some(*seconds)),
        RunTimeout::Plain(TimeoutValue::String(text)) => parse_timeout_text(text),
        RunTimeout::Structured(structured) => Ok(Some(structured.read.unwrap_or(600.0_f64))),
    }
}

// float(" 7.5 ") is 7.5, and a non-numeric string raises ValueError before the provider call
fn parse_timeout_text(text: &str) -> Result<Option<f64>, AssistantRunError> {
    let seconds =
        text.trim()
            .parse::<f64>()
            .map_err(|_invalid| AssistantRunError::InvalidTimeoutText {
                text: text.to_owned(),
            })?;
    Ok(Some(seconds))
}

// azure_ad_token = extra_body.pop("azure_ad_token", None), or the AZURE_AD_TOKEN secret when extra_body is null
pub(super) fn facade_token(
    call: &RunThreadCall,
    settings: &ResourceSdkSettings,
) -> Result<Option<SecretString>, AssistantRunError> {
    let Some(extra_body) = call.params.extra_body.as_ref() else {
        return Ok(None);
    };
    match extra_body {
        JsonValue::Null => {
            let token = settings
                .azure_ad_token
                .as_ref()
                .filter(|token| !token.expose_secret().is_empty());
            Ok(token.cloned())
        }
        JsonValue::Object(body) => Ok(body.get("azure_ad_token").and_then(token_text)),
        JsonValue::Array(_) | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {
            Err(AssistantRunError::InvalidExtraBody)
        }
    }
}

// str(azure_ad_token): a present null pops to None and any other value becomes its Python text
fn token_text(token: &JsonValue) -> Option<SecretString> {
    match token {
        JsonValue::Null => None,
        JsonValue::String(text) => Some(SecretString::from(text.clone())),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Some(SecretString::from(token.to_string()))
        }
    }
}

impl From<&RunParams> for ResourceSdkRequestOptions {
    fn from(params: &RunParams) -> Self {
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
