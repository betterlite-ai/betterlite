use std::collections::BTreeMap;

use betterlite_config::sections::batch_provider_create::Settings;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use secrecy::SecretString;

use crate::api::batch_provider_create::error::BatchProviderCreateError;
use crate::api::batch_provider_create::{
    BatchProvider, BatchProviderCreateCall, BatchProviderCreateInput, DEFAULT_OPENAI_BASE,
    ProviderBatchCall, ProviderConfigCall, VertexCredentials,
};

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
    let request = &input.optional_params;
    match provider {
        BatchProvider::Openai | BatchProvider::HostedVllm | BatchProvider::LitellmProxy => {
            Resolved {
                api_base: [
                    request.api_base.as_deref(),
                    settings.api_base.as_deref(),
                    settings.openai_base_url.as_deref(),
                    settings.openai_api_base.as_deref(),
                ]
                .into_iter()
                .flatten()
                .next()
                .map(str::to_owned),
                api_key: [
                    request.api_key.as_ref(),
                    settings.api_key.as_ref(),
                    settings.openai_key.as_ref(),
                    settings.openai_api_key.as_ref(),
                ]
                .into_iter()
                .flatten()
                .next()
                .cloned(),
                organization: [
                    request.organization.as_deref(),
                    settings.organization.as_deref(),
                    settings.openai_organization.as_deref(),
                ]
                .into_iter()
                .flatten()
                .next()
                .map(str::to_owned),
                ..Resolved::empty()
            }
        }
        BatchProvider::Azure => Resolved {
            api_base: [
                request.api_base.as_deref(),
                settings.api_base.as_deref(),
                settings.azure_api_base.as_deref(),
            ]
            .into_iter()
            .flatten()
            .next()
            .map(str::to_owned),
            api_version: [
                request.api_version.as_deref(),
                settings.api_version.as_deref(),
                settings.azure_api_version.as_deref(),
            ]
            .into_iter()
            .flatten()
            .next()
            .map(str::to_owned),
            api_key: [
                request.api_key.as_ref(),
                settings.api_key.as_ref(),
                settings.azure_key.as_ref(),
                settings.azure_openai_api_key.as_ref(),
                settings.azure_api_key.as_ref(),
            ]
            .into_iter()
            .flatten()
            .next()
            .cloned(),
            ..Resolved::empty()
        },
        BatchProvider::VertexAi => Resolved {
            api_base: Some(request.api_base.clone().unwrap_or_default()),
            vertex_project: [
                request.vertex_project.as_deref(),
                settings.vertex_project.as_deref(),
                settings.vertexai_project.as_deref(),
            ]
            .into_iter()
            .flatten()
            .next()
            .map(str::to_owned),
            vertex_location: [
                request.vertex_location.as_deref(),
                settings.vertex_location.as_deref(),
                settings.vertexai_location.as_deref(),
            ]
            .into_iter()
            .flatten()
            .next()
            .map(str::to_owned),
            vertex_credentials: request.vertex_credentials.clone().or_else(|| {
                settings
                    .vertexai_credentials
                    .clone()
                    .map(VertexCredentials::Path)
            }),
            ..Resolved::empty()
        },
        BatchProvider::Xai => Resolved {
            api_base: request.api_base.clone(),
            api_key: request.api_key.clone(),
            ..Resolved::empty()
        },
        BatchProvider::Bedrock | BatchProvider::Mistral => Resolved::empty(),
    }
}

pub(super) fn build_call(
    input: &BatchProviderCreateInput,
    model: Option<&str>,
    settings: &Settings,
    timeout: f64,
    body: CreateBatchRequest,
) -> Result<BatchProviderCreateCall, BatchProviderCreateError> {
    if let Some(model) = model
        && matches!(
            input.custom_llm_provider,
            BatchProvider::Bedrock | BatchProvider::Mistral
        )
    {
        return Ok(BatchProviderCreateCall::ProviderConfig(
            ProviderConfigCall {
                provider: input.custom_llm_provider,
                model: model.to_owned(),
                headers: match &input.extra_headers {
                    None => BTreeMap::new(),
                    Some(headers) => headers.clone().into_iter().collect(),
                },
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
        api_base: match input.custom_llm_provider {
            BatchProvider::Openai | BatchProvider::HostedVllm | BatchProvider::LitellmProxy => {
                Some(
                    resolved
                        .api_base
                        .unwrap_or_else(|| DEFAULT_OPENAI_BASE.to_owned()),
                )
            }
            BatchProvider::VertexAi => Some(resolved.api_base.unwrap_or_default()),
            _other => resolved.api_base,
        },
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
