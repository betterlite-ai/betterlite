use std::num::TryFromIntError;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_azure::azure_assistant_definitions::operations::ClientRequest;
use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::credentials::source::CredentialSources;
use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::{azure_endpoints, resource_sdk_settings};
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, resolve,
};
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::assistants::records::AssistantCreate;
use betterlite_openai::batches::sdk::TimeoutPhases;
use secrecy::SecretString;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, request_timeout,
    resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::call::{AssistantCreateCall, AzureClientParams, AzureCreateCall, OpenAiCreateCall};
use super::{AssistantCreateInput, ClientClass, CreateTimeout};

// every way the façade's own argument handling can refuse a create_assistants call
#[derive(Debug, thiserror::Error)]
pub(super) enum ProjectionError {
    #[error(transparent)]
    Resolve(#[from] ResolutionError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("the timeout {seconds} seconds is not a duration")]
    Duration {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("max_retries must be a non-negative integer, got {value}")]
    Retries {
        value: i64,
        #[source]
        source: TryFromIntError,
    },
}

// get_llm_provider prepends the declared provider the model doesn't name, then reads the provider back.
pub(super) fn provider(
    input: &AssistantCreateInput,
    resolution: &ResolutionContext<'_>,
) -> Result<String, ProjectionError> {
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
    Ok(resolve(&request, resolution)?.provider.slug().to_owned())
}

// optional_params.timeout or request_timeout or 600, where a zero or an empty value falls through
pub(super) fn timeout(
    input: &AssistantCreateInput,
    provider: &str,
) -> Result<FineTuningTimeout, ProjectionError> {
    match input.timeout.as_ref() {
        Some(CreateTimeout::Phases(phases)) => Ok(resolve_fine_tuning_timeout(
            None,
            provider,
            Some(structured(phases)),
        )?),
        Some(CreateTimeout::Seconds(seconds)) if *seconds != 0.0_f64 => Ok(
            resolve_fine_tuning_timeout(Some(&TimeoutValue::Number(*seconds)), provider, None)?,
        ),
        Some(CreateTimeout::Text(text)) if !text.is_empty() => Ok(resolve_fine_tuning_timeout(
            Some(&TimeoutValue::String(text.clone())),
            provider,
            None,
        )?),
        None | Some(CreateTimeout::Seconds(_) | CreateTimeout::Text(_)) => Ok(
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
pub(super) fn deadline(timeout: FineTuningTimeout) -> Result<Duration, ProjectionError> {
    let seconds = seconds(timeout);
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| ProjectionError::Duration { seconds, source })
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

pub(super) fn retries(max_retries: Option<i64>) -> Result<Option<u32>, ProjectionError> {
    max_retries
        .map(|value| {
            u32::try_from(value).map_err(|source| ProjectionError::Retries { value, source })
        })
        .transpose()
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CallKind {
    OpenAi,
    Azure,
}

pub(super) struct CallRequest<'request> {
    pub kind: CallKind,
    pub settings: &'request resource_sdk_settings::Settings,
    pub environment: &'request (dyn EnvironmentLookup + Sync),
    pub input: &'request AssistantCreateInput,
    pub timeout: FineTuningTimeout,
    pub is_async: bool,
}

pub(super) fn call(request: CallRequest<'_>) -> AssistantCreateCall {
    let parts = prepare(request.input);
    let resolved = ResourceSdkSettings::from(request.settings);
    match request.kind {
        CallKind::OpenAi => AssistantCreateCall::OpenAi(Box::new(OpenAiCreateCall {
            create: parts.create,
            api_base: resolved.openai_base(&parts.options),
            api_key: resolved.openai_api_key(&parts.options).cloned(),
            organization: resolved
                .openai_organization(&parts.options)
                .map(str::to_owned),
            timeout: request.timeout,
            max_retries: request.input.max_retries,
            client: request.input.client,
            is_async: request.is_async,
        })),
        CallKind::Azure => AssistantCreateCall::Azure(Box::new(AzureCreateCall {
            create: parts.create,
            api_base: resolved.azure_base(&parts.options).map(str::to_owned),
            api_key: resolved.azure_api_key(&parts.options).cloned(),
            api_version: resolved.azure_version(&parts.options).map(str::to_owned),
            azure_ad_token: azure_ad_token(request.input, request.environment),
            timeout: request.timeout,
            max_retries: request.input.max_retries,
            client: effective_client(request.input.client),
            is_async: request.is_async,
            litellm_params: parts.litellm,
        })),
    }
}

// B519 builds the SDK client from these options, so the deadline and retry count are refused here.
pub(super) fn openai_options(call: &OpenAiCreateCall) -> Result<ClientOptions, ProjectionError> {
    Ok(ClientOptions {
        api_key: call.api_key.clone(),
        api_base: Some(call.api_base.clone()),
        timeout: Some(deadline(call.timeout)?),
        max_retries: retries(call.max_retries)?,
        organization: call.organization.clone(),
    })
}

// The façade's own token, timeout and retries stay beside the B176 params, as get_litellm_params split them.
pub(super) fn azure_request<'settings>(
    settings: &'settings azure_endpoints::Settings,
    environment: &'settings (dyn EnvironmentLookup + Sync),
    sources: &'settings (dyn CredentialSources + Sync),
    call: &AzureCreateCall,
) -> ClientRequest<'settings> {
    ClientRequest {
        params: LiteLlmParams {
            azure_ad_token: call.litellm_params.azure_ad_token.clone(),
            auth: call.litellm_params.auth.clone(),
            max_retries: call.litellm_params.max_retries,
            timeout: call.litellm_params.timeout,
        },
        provider: None,
        api_key: call.api_key.clone(),
        api_base: call.api_base.clone(),
        api_version: call.api_version.clone(),
        azure_ad_token: call.azure_ad_token.clone(),
        timeout: seconds(call.timeout),
        max_retries: call.max_retries,
        refresh_enabled: false,
        session: SessionKind::Dedicated,
        settings,
        environment,
        sources,
    }
}

// The env token is read only when extra_body is explicitly null; an absent or empty mapping reads none.
fn azure_ad_token(
    input: &AssistantCreateInput,
    environment: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    match &input.extra_body {
        Some(None) => environment.lookup("AZURE_AD_TOKEN").map(SecretString::from),
        Some(Some(body)) => body.get("azure_ad_token").cloned().map(SecretString::from),
        None => None,
    }
}

// isinstance(client, OpenAI) also matches AzureOpenAI; AsyncOpenAI sits outside that hierarchy.
const fn effective_client(client: Option<ClientClass>) -> Option<ClientClass> {
    match client {
        Some(ClientClass::OpenAi | ClientClass::AzureOpenAi) | None => None,
        Some(other) => Some(other),
    }
}

struct Prepared {
    create: AssistantCreate,
    options: ResourceSdkRequestOptions,
    litellm: AzureClientParams,
}

// What create_assistants, the request options and get_litellm_params each copy out of the same input.
fn prepare(input: &AssistantCreateInput) -> Prepared {
    Prepared {
        create: AssistantCreate {
            model: input.model.clone(),
            description: input.description.clone().map(Some),
            instructions: input.instructions.clone().map(Some),
            metadata: input.metadata.clone().map(Some),
            name: input.name.clone().map(Some),
            reasoning_effort: None,
            response_format: input.response_format.clone().map(Some),
            temperature: input.temperature.clone().map(Some),
            tool_resources: input.tool_resources.clone().map(Some),
            tools: input.tools.clone().map(Some),
            top_p: input.top_p.clone().map(Some),
        },
        options: ResourceSdkRequestOptions {
            api_base: input.api_base.clone(),
            api_version: input.api_version.clone(),
            api_key: input.api_key.clone(),
            organization: input.organization.clone(),
            vertex_project: None,
            vertex_location: None,
            vertex_credentials: None,
        },
        litellm: AzureClientParams {
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
        },
    }
}

const fn raw_timeout(timeout: Option<&CreateTimeout>) -> Option<f64> {
    match timeout {
        Some(CreateTimeout::Seconds(seconds)) => Some(*seconds),
        Some(CreateTimeout::Text(_) | CreateTimeout::Phases(_)) | None => None,
    }
}
