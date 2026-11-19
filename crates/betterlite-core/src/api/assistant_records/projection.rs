use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_azure::azure_urls::AuthParams;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::resource_sdk_settings;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_openai::assistants::records::AssistantCreate;
use betterlite_openai::batches::sdk::TimeoutPhases;
use secrecy::SecretString;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, request_timeout,
    resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::call::{AzureClientParams, AzureCreateCall, OpenAiCreateCall};
use super::{AssistantCreateInput, ClientClass, CreateTimeout};

// get_llm_provider prepends the declared provider the model doesn't name, then reads the provider back.
pub(super) fn provider(
    input: &AssistantCreateInput,
    resolution: &ResolutionContext<'_>,
) -> Result<String, String> {
    let prefixed = format!("{}/{}", input.provider, input.model);
    let model = if input.model.starts_with(&format!("{}/", input.provider)) {
        input.model.as_str()
    } else {
        prefixed.as_str()
    };
    let request = ResolutionRequest {
        model,
        custom_llm_provider: Some(input.provider.as_str()),
        api_base: None,
        api_key: None,
    };
    resolve(&request, resolution)
        .map(|resolved| resolved.provider.slug().to_owned())
        .map_err(|source| source.to_string())
}

// optional_params.timeout or request_timeout or 600, where a zero or an empty value falls through
pub(super) fn timeout(
    input: &AssistantCreateInput,
    provider: &str,
) -> Result<FineTuningTimeout, TimeoutError> {
    match input.timeout.as_ref() {
        Some(CreateTimeout::Phases(phases)) => {
            resolve_fine_tuning_timeout(None, provider, Some(structured(phases)))
        }
        Some(CreateTimeout::Seconds(seconds)) if *seconds != 0.0_f64 => {
            resolve_fine_tuning_timeout(Some(&TimeoutValue::Number(*seconds)), provider, None)
        }
        Some(CreateTimeout::Text(text)) if !text.is_empty() => {
            resolve_fine_tuning_timeout(Some(&TimeoutValue::String(text.clone())), provider, None)
        }
        None | Some(CreateTimeout::Seconds(_)) | Some(CreateTimeout::Text(_)) => Ok(
            FineTuningTimeout::Seconds(request_timeout(None, input.request_timeout)?),
        ),
    }
}

const fn structured(phases: &TimeoutPhases) -> StructuredTimeout {
    StructuredTimeout {
        connect: phases.connect,
        read: phases.read,
        write: phases.write,
        pool: phases.pool,
    }
}

// An httpx.Timeout reaches the SDK whole; the client this port builds bounds the call by its read phase.
pub(super) fn deadline(timeout: FineTuningTimeout) -> Result<Duration, String> {
    let seconds = seconds(timeout);
    Duration::try_from_secs_f64(seconds)
        .map_err(|_source| format!("the timeout {seconds} seconds is not a duration"))
}

pub(super) fn seconds(timeout: FineTuningTimeout) -> f64 {
    match timeout {
        FineTuningTimeout::Seconds(seconds) => seconds,
        FineTuningTimeout::Structured(phases) => phases
            .read
            .filter(|read| *read != 0.0_f64)
            .unwrap_or(600.0_f64),
    }
}

pub(super) fn retries(max_retries: Option<i64>) -> Result<Option<u32>, String> {
    max_retries
        .map(|value| {
            u32::try_from(value).map_err(|_source| {
                format!("max_retries must be a non-negative integer, got {value}")
            })
        })
        .transpose()
}

pub(super) fn openai(
    settings: &resource_sdk_settings::Settings,
    input: &AssistantCreateInput,
    timeout: FineTuningTimeout,
    is_async: bool,
) -> OpenAiCreateCall {
    let options = ResourceSdkRequestOptions::from(input);
    let resolved = ResourceSdkSettings::from(settings);
    OpenAiCreateCall {
        create: create(input),
        api_base: resolved.openai_base(&options),
        api_key: resolved.openai_api_key(&options).cloned(),
        organization: resolved.openai_organization(&options).map(str::to_owned),
        timeout,
        max_retries: input.max_retries,
        client: input.client,
        is_async,
    }
}

pub(super) fn azure(
    settings: &resource_sdk_settings::Settings,
    environment: &(dyn EnvironmentLookup + Sync),
    input: &AssistantCreateInput,
    timeout: FineTuningTimeout,
    is_async: bool,
) -> AzureCreateCall {
    let options = ResourceSdkRequestOptions::from(input);
    let resolved = ResourceSdkSettings::from(settings);
    AzureCreateCall {
        create: create(input),
        api_base: resolved.azure_base(&options).map(str::to_owned),
        api_key: resolved.azure_api_key(&options).cloned(),
        api_version: resolved.azure_version(&options).map(str::to_owned),
        azure_ad_token: azure_ad_token(&input.extra_body, environment),
        timeout,
        max_retries: input.max_retries,
        client: effective_client(input.client),
        is_async,
        litellm_params: litellm_params(input),
    }
}

// The env token is read only when extra_body is explicitly null; an absent or empty mapping reads none.
fn azure_ad_token(
    extra_body: &Option<Option<BTreeMap<String, String>>>,
    environment: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    match extra_body {
        Some(None) => environment.lookup("AZURE_AD_TOKEN").map(SecretString::from),
        Some(Some(body)) => body.get("azure_ad_token").cloned().map(SecretString::from),
        None => None,
    }
}

// isinstance(client, OpenAI) also matches AzureOpenAI; AsyncOpenAI sits outside that hierarchy.
fn effective_client(client: Option<ClientClass>) -> Option<ClientClass> {
    match client {
        Some(ClientClass::OpenAi | ClientClass::AzureOpenAi) => None,
        Some(other) => Some(other),
        None => None,
    }
}

fn create(input: &AssistantCreateInput) -> AssistantCreate {
    AssistantCreate {
        model: input.model.clone(),
        description: input.description.clone().map(Some),
        instructions: input.instructions.clone().map(Some),
        metadata: input.metadata.clone().map(Some),
        name: input.name.clone().map(Some),
        reasoning_effort: None,
        response_format: input.response_format.clone().map(Some),
        temperature: input.temperature,
        tool_resources: input.tool_resources.clone().map(Some),
        tools: input.tools.clone().map(Some),
        top_p: input.top_p,
    }
}

// get_litellm_params copies the caller's timeout through as it arrived; only a number reaches a duration.
fn litellm_params(input: &AssistantCreateInput) -> AzureClientParams {
    AzureClientParams {
        azure_ad_token: input.azure_ad_token.clone(),
        auth: AuthParams {
            tenant_id: input.tenant_id.clone(),
            client_id: input.client_id.clone(),
            client_secret: input.client_secret.clone(),
            azure_username: input.azure_username.clone(),
            azure_password: input.azure_password.clone(),
            azure_scope: input.azure_scope.clone(),
        },
        max_retries: input.max_retries,
        timeout: raw_timeout(input.timeout.as_ref()),
    }
}

fn raw_timeout(timeout: Option<&CreateTimeout>) -> Option<f64> {
    match timeout {
        Some(CreateTimeout::Seconds(seconds)) => Some(*seconds),
        Some(CreateTimeout::Text(_) | CreateTimeout::Phases(_)) | None => None,
    }
}

impl From<&AssistantCreateInput> for ResourceSdkRequestOptions {
    fn from(input: &AssistantCreateInput) -> Self {
        Self {
            api_base: input.api_base.clone(),
            api_version: input.api_version.clone(),
            api_key: input.api_key.clone(),
            organization: input.organization.clone(),
            ..Self::default()
        }
    }
}
