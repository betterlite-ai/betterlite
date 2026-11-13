use std::collections::BTreeMap;

use betterlite_config::sections::batch_provider_create::Settings;
use betterlite_openai::api::batch_records::CreateBatchRequest;

use crate::api::batch_provider_create::error::BatchProviderCreateError;
use crate::api::batch_provider_create::{
    AzureBatchCall, BatchProvider, BatchProviderCreateCall, BatchProviderCreateInput,
    DEFAULT_OPENAI_BASE, OpenAiBatchCall, ProviderConfigCall, VertexBatchCall, VertexCredentials,
    XaiBatchCall,
};

// ProviderConfigManager.get_provider_batches_config answers for these two, and create_batch only asks when a model is set
const fn has_provider_config(provider: BatchProvider) -> bool {
    matches!(provider, BatchProvider::Bedrock | BatchProvider::Mistral)
}

pub(super) fn build_call(
    input: &BatchProviderCreateInput,
    model: Option<&str>,
    settings: &Settings,
    timeout: f64,
    body: CreateBatchRequest,
) -> Result<BatchProviderCreateCall, BatchProviderCreateError> {
    if let Some(model) = model
        && has_provider_config(input.custom_llm_provider)
    {
        return Ok(BatchProviderCreateCall::ProviderConfig(provider_config_call(
            input, model, timeout, body,
        )));
    }
    match input.custom_llm_provider {
        BatchProvider::Xai => Ok(BatchProviderCreateCall::Xai(xai_call(
            input, timeout, body,
        ))),
        BatchProvider::Openai | BatchProvider::HostedVllm | BatchProvider::LitellmProxy => Ok(
            BatchProviderCreateCall::OpenAiCompatible(openai_call(input, settings, timeout, body)),
        ),
        BatchProvider::Azure => Ok(BatchProviderCreateCall::Azure(azure_call(
            input, settings, timeout, body,
        ))),
        BatchProvider::VertexAi => Ok(BatchProviderCreateCall::VertexAi(vertex_call(
            input, settings, timeout, body,
        ))),
        BatchProvider::Bedrock | BatchProvider::Mistral => {
            Err(BatchProviderCreateError::UnsupportedProvider {
                provider: input.custom_llm_provider,
            })
        }
    }
}

fn provider_config_call(
    input: &BatchProviderCreateInput,
    model: &str,
    timeout: f64,
    body: CreateBatchRequest,
) -> ProviderConfigCall {
    ProviderConfigCall {
        provider: input.custom_llm_provider,
        model: model.to_owned(),
        body,
        headers: match &input.extra_headers {
            None => BTreeMap::new(),
            Some(headers) => headers.clone().into_iter().collect(),
        },
        api_base: input.optional_params.api_base.clone(),
        api_key: input.optional_params.api_key.clone(),
        timeout,
        is_async: input.is_async,
    }
}

fn xai_call(input: &BatchProviderCreateInput, timeout: f64, body: CreateBatchRequest) -> XaiBatchCall {
    XaiBatchCall {
        body,
        api_base: input.optional_params.api_base.clone(),
        api_key: input.optional_params.api_key.clone(),
        timeout,
        is_async: input.is_async,
    }
}

fn openai_call(
    input: &BatchProviderCreateInput,
    settings: &Settings,
    timeout: f64,
    body: CreateBatchRequest,
) -> OpenAiBatchCall {
    let optional = &input.optional_params;
    OpenAiBatchCall {
        body,
        api_base: optional
            .api_base
            .clone()
            .or_else(|| settings.api_base.clone())
            .or_else(|| settings.openai_base_url.clone())
            .or_else(|| settings.openai_api_base.clone())
            .unwrap_or_else(|| DEFAULT_OPENAI_BASE.to_owned()),
        api_key: optional
            .api_key
            .clone()
            .or_else(|| settings.api_key.clone())
            .or_else(|| settings.openai_key.clone())
            .or_else(|| settings.openai_api_key.clone()),
        organization: optional
            .organization
            .clone()
            .or_else(|| settings.organization.clone())
            .or_else(|| settings.openai_organization.clone()),
        timeout,
        max_retries: optional.max_retries,
        is_async: input.is_async,
    }
}

fn azure_call(
    input: &BatchProviderCreateInput,
    settings: &Settings,
    timeout: f64,
    body: CreateBatchRequest,
) -> AzureBatchCall {
    let optional = &input.optional_params;
    AzureBatchCall {
        body,
        api_base: optional
            .api_base
            .clone()
            .or_else(|| settings.api_base.clone())
            .or_else(|| settings.azure_api_base.clone()),
        api_key: optional
            .api_key
            .clone()
            .or_else(|| settings.api_key.clone())
            .or_else(|| settings.azure_key.clone())
            .or_else(|| settings.azure_openai_api_key.clone())
            .or_else(|| settings.azure_api_key.clone()),
        api_version: optional
            .api_version
            .clone()
            .or_else(|| settings.api_version.clone())
            .or_else(|| settings.azure_api_version.clone()),
        timeout,
        max_retries: optional.max_retries,
        is_async: input.is_async,
    }
}

fn vertex_call(
    input: &BatchProviderCreateInput,
    settings: &Settings,
    timeout: f64,
    body: CreateBatchRequest,
) -> VertexBatchCall {
    let optional = &input.optional_params;
    VertexBatchCall {
        body,
        api_base: optional.api_base.clone().unwrap_or_default(),
        vertex_project: optional
            .vertex_project
            .clone()
            .or_else(|| settings.vertex_project.clone())
            .or_else(|| settings.vertexai_project.clone()),
        vertex_location: optional
            .vertex_location
            .clone()
            .or_else(|| settings.vertex_location.clone())
            .or_else(|| settings.vertexai_location.clone()),
        vertex_credentials: optional.vertex_credentials.clone().or_else(|| {
            settings
                .vertexai_credentials
                .clone()
                .map(VertexCredentials::Path)
        }),
        timeout,
        max_retries: optional.max_retries,
        custom_endpoint: optional.custom_endpoint,
        is_async: input.is_async,
    }
}
