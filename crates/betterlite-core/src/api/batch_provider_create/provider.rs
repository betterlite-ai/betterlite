use std::collections::BTreeMap;

use betterlite_config::sections::batch_provider_create::Settings;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use secrecy::SecretString;

use crate::api::batch_provider_create::error::BatchProviderCreateError;
use crate::api::batch_provider_create::{
    BatchOptionalParams, BatchProvider, BatchProviderCreateCall, BatchProviderCreateInput,
    ProviderBatchCall, ProviderConfigCall, VertexCredentials,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

// the credentials each provider branch falls back through, resolved in LiteLLM's order
struct Resolved {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    api_version: Option<String>,
    organization: Option<String>,
    vertex_project: Option<String>,
    vertex_location: Option<String>,
    vertex_credentials: Option<VertexCredentials>,
}

impl Resolved {
    const fn empty() -> Self {
        Self {
            api_base: None,
            api_key: None,
            api_version: None,
            organization: None,
            vertex_project: None,
            vertex_location: None,
            vertex_credentials: None,
        }
    }
}

fn resolve(
    provider: BatchProvider,
    input: &BatchProviderCreateInput,
    settings: &Settings,
) -> Resolved {
    let resources = resource_settings(settings);
    let options = request_options(&input.optional_params);
    let mut resolved = Resolved::empty();
    match provider {
        // OPENAI_COMPATIBLE_BATCH_AND_FILES_PROVIDERS shares one branch, whose base always resolves
        BatchProvider::Openai | BatchProvider::HostedVllm | BatchProvider::LitellmProxy => {
            resolved.api_base = Some(resources.openai_base(&options));
            resolved.api_key = resources.openai_api_key(&options).cloned();
            resolved.organization = resources.openai_organization(&options).map(str::to_owned);
        }
        BatchProvider::Azure => {
            resolved.api_base = resources.azure_base(&options).map(str::to_owned);
            resolved.api_version = resources.azure_version(&options).map(str::to_owned);
            resolved.api_key = resources.azure_api_key(&options).cloned();
        }
        BatchProvider::VertexAi => {
            let requested = &input.optional_params.vertex_credentials;
            resolved.api_base = Some(options.api_base.clone().unwrap_or_default());
            resolved.vertex_project = resources.vertex_project(&options).map(str::to_owned);
            resolved.vertex_location = resources.vertex_location(&options).map(str::to_owned);
            resolved.vertex_credentials = requested.clone().or_else(|| {
                resources
                    .vertex_credentials(&options)
                    .map(|secret| VertexCredentials::Text(secret.clone()))
            });
        }
        // xai reads only what the caller passes; no litellm_settings key falls back into it
        BatchProvider::Xai => {
            resolved.api_base.clone_from(&options.api_base);
            resolved.api_key.clone_from(&options.api_key);
        }
        BatchProvider::Bedrock | BatchProvider::Mistral => {}
    }
    resolved
}

// the litellm_settings keys create_batch reads, under the names the config loads them as
fn resource_settings(settings: &Settings) -> ResourceSdkSettings {
    ResourceSdkSettings {
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

// the request side of the same fallback; its vertex_credentials stay out because they may be fields
fn request_options(params: &BatchOptionalParams) -> ResourceSdkRequestOptions {
    ResourceSdkRequestOptions {
        api_base: params.api_base.clone(),
        api_version: params.api_version.clone(),
        api_key: params.api_key.clone(),
        organization: params.organization.clone(),
        vertex_project: params.vertex_project.clone(),
        vertex_location: params.vertex_location.clone(),
        vertex_credentials: None,
    }
}

pub(super) fn build_call(
    input: &BatchProviderCreateInput,
    model: Option<&str>,
    settings: &Settings,
    timeout: f64,
    body: CreateBatchRequest,
) -> Result<BatchProviderCreateCall, BatchProviderCreateError> {
    if let Some(resolved) = model
        && matches!(
            input.custom_llm_provider,
            BatchProvider::Bedrock | BatchProvider::Mistral
        )
    {
        return Ok(BatchProviderCreateCall::ProviderConfig(
            ProviderConfigCall {
                provider: input.custom_llm_provider,
                model: resolved.to_owned(),
                headers: input
                    .extra_headers
                    .as_ref()
                    .map_or_else(BTreeMap::new, |headers| {
                        headers.clone().into_iter().collect()
                    }),
                api_base: input.optional_params.api_base.clone(),
                api_key: input.optional_params.api_key.clone(),
                body,
                timeout,
                is_async: input.is_async,
            },
        ));
    }
    provider_call(input, settings, timeout, body)
}

fn provider_call(
    input: &BatchProviderCreateInput,
    settings: &Settings,
    timeout: f64,
    body: CreateBatchRequest,
) -> Result<BatchProviderCreateCall, BatchProviderCreateError> {
    let provider = input.custom_llm_provider;
    let resolved = resolve(provider, input, settings);
    let parts = batch_parts(input, resolved, timeout, body);
    match provider {
        BatchProvider::Xai => Ok(BatchProviderCreateCall::Xai(parts)),
        BatchProvider::Openai | BatchProvider::HostedVllm | BatchProvider::LitellmProxy => {
            Ok(BatchProviderCreateCall::OpenAiCompatible(parts))
        }
        BatchProvider::Azure => Ok(BatchProviderCreateCall::Azure(parts)),
        BatchProvider::VertexAi => Ok(BatchProviderCreateCall::VertexAi(parts)),
        BatchProvider::Bedrock | BatchProvider::Mistral => {
            Err(BatchProviderCreateError::UnsupportedProvider { provider })
        }
    }
}

fn batch_parts(
    input: &BatchProviderCreateInput,
    resolved: Resolved,
    timeout: f64,
    body: CreateBatchRequest,
) -> ProviderBatchCall {
    ProviderBatchCall {
        api_base: resolved.api_base,
        body,
        api_key: resolved.api_key,
        organization: resolved.organization,
        api_version: resolved.api_version,
        vertex_project: resolved.vertex_project,
        vertex_location: resolved.vertex_location,
        vertex_credentials: resolved.vertex_credentials,
        custom_endpoint: input.optional_params.custom_endpoint,
        timeout,
        max_retries: input.optional_params.max_retries,
        is_async: input.is_async,
    }
}
