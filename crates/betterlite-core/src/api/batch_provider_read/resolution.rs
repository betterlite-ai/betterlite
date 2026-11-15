use std::collections::BTreeMap;

use betterlite_config::sections::batch_provider_read::Settings;
use betterlite_openai::api::batch_records::RetrieveBatchRequest;
use betterlite_openai::batches::sdk::Timeout;
use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};

use super::call::{
    AnthropicBatchReadCall, AwsCredentials, AzureBatchReadCall, AzureReadParams,
    OpenAiBatchReadCall, ProviderConfigReadCall, VertexBatchReadCall, VertexReadCredentials,
};
use super::error::ReadCallError;
use super::{BatchProviderReadInput, BatchReadOptionalParams};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

// base_llm_http_handler.retrieve_batch, which the configured Bedrock and Mistral providers reach
pub(super) fn provider_config(
    input: &BatchProviderReadInput,
    model: &str,
    timeout: Timeout,
    aws: AwsCredentials,
) -> ProviderConfigReadCall {
    ProviderConfigReadCall {
        provider: input.provider.clone(),
        model: model.to_owned(),
        batch_id: input.batch_id.clone(),
        headers: BTreeMap::from_iter(input.extra_headers.clone().unwrap_or_default()),
        api_base: input.optional_params.api_base.clone(),
        api_key: input.optional_params.api_key.clone(),
        aws,
        timeout,
        is_async: input.aretrieve_batch,
    }
}

// OpenAIBatchesAPI.retrieve_batch
pub(super) fn openai(
    input: &BatchProviderReadInput,
    settings: &Settings,
    timeout: Timeout,
) -> OpenAiBatchReadCall {
    let resources = ResourceSdkSettings::from(&settings.resources);
    let options = request_options(&input.optional_params);
    OpenAiBatchReadCall {
        request: RetrieveBatchRequest {
            batch_id: Some(input.batch_id.clone()),
            extra_headers: input.extra_headers.clone(),
            extra_body: input.extra_body.clone(),
            timeout: None,
        },
        api_base: resources.openai_base(&options),
        api_key: resources.openai_api_key(&options).cloned(),
        organization: resources.openai_organization(&options).map(str::to_owned),
        timeout,
        max_retries: input.optional_params.max_retries,
        is_async: input.aretrieve_batch,
    }
}

// AzureBatchesAPI.retrieve_batch
pub(super) fn azure(
    input: &BatchProviderReadInput,
    settings: &Settings,
    timeout: Timeout,
) -> AzureBatchReadCall {
    let params = &input.optional_params;
    let resources = ResourceSdkSettings::from(&settings.resources);
    let options = request_options(params);
    AzureBatchReadCall {
        request: RetrieveBatchRequest {
            batch_id: Some(input.batch_id.clone()),
            extra_headers: input.extra_headers.clone(),
            extra_body: input.extra_body.clone(),
            timeout: None,
        },
        api_base: resources.azure_base(&options).map(str::to_owned),
        api_key: resources.azure_api_key(&options).cloned(),
        api_version: resources.azure_version(&options).map(str::to_owned),
        timeout,
        max_retries: params.max_retries,
        litellm_params: AzureReadParams {
            api_key: params.api_key.clone(),
            api_base: params.api_base.clone(),
            api_version: params.api_version.clone(),
            max_retries: params.max_retries,
            timeout: params.timeout.clone(),
            azure_ad_token: params.azure_ad_token.clone(),
            tenant_id: params.tenant_id.clone(),
            client_id: params.client_id.clone(),
            client_secret: params.client_secret.clone(),
            azure_scope: params.azure_scope.clone(),
            azure_username: params.azure_username.clone(),
            azure_password: params.azure_password.clone(),
        },
        is_async: input.aretrieve_batch,
    }
}

// VertexAIBatchPrediction.retrieve_batch
pub(super) fn vertex(
    input: &BatchProviderReadInput,
    settings: &Settings,
    timeout: Timeout,
) -> Result<VertexBatchReadCall, ReadCallError> {
    let resources = ResourceSdkSettings::from(&settings.resources);
    let options = request_options(&input.optional_params);
    Ok(VertexBatchReadCall {
        batch_id: input.batch_id.clone(),
        api_base: input.optional_params.api_base.clone().unwrap_or_default(),
        vertex_project: resources.vertex_project(&options).map(str::to_owned),
        vertex_location: resources.vertex_location(&options).map(str::to_owned),
        vertex_credentials: resolve_vertex_credentials(
            &resources,
            &options,
            &input.optional_params,
        )?,
        timeout,
        max_retries: input.optional_params.max_retries,
        is_async: input.aretrieve_batch,
    })
}

// LiteLLM declares vertex_credentials as a path string or the inline service-account fields
fn resolve_vertex_credentials(
    resources: &ResourceSdkSettings,
    options: &ResourceSdkRequestOptions,
    params: &BatchReadOptionalParams,
) -> Result<Option<VertexReadCredentials>, ReadCallError> {
    match params.vertex_credentials.as_ref() {
        Some(Json::Object(account)) => {
            Ok(Some(VertexReadCredentials::ServiceAccount(account.clone())))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(ReadCallError::InvalidVertexCredentials)
        }
        Some(Json::Text(_) | Json::Null) | None => Ok(resources
            .vertex_credentials(options)
            .cloned()
            .map(VertexReadCredentials::Path)),
    }
}

// AnthropicBatchesHandler.retrieve_batch
pub(super) fn anthropic(
    input: &BatchProviderReadInput,
    settings: &Settings,
    timeout: Timeout,
) -> AnthropicBatchReadCall {
    let params = &input.optional_params;
    AnthropicBatchReadCall {
        batch_id: input.batch_id.clone(),
        api_base: first_text([
            params.api_base.as_deref(),
            settings.resources.api_base.as_deref(),
            settings.anthropic_base(),
        ]),
        // the branch falls back through litellm.azure_key as well, a copy-paste the source keeps
        api_key: first_secret([
            params.api_key.as_ref(),
            settings.resources.api_key.as_ref(),
            settings.resources.azure_key.as_ref(),
            settings.anthropic_api_key.as_ref(),
        ]),
        timeout,
        max_retries: params.max_retries,
        is_async: input.aretrieve_batch,
    }
}

fn first_text(candidates: [Option<&str>; 3]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

fn first_secret(candidates: [Option<&SecretString>; 4]) -> Option<SecretString> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
        .cloned()
}

fn request_options(params: &BatchReadOptionalParams) -> ResourceSdkRequestOptions {
    ResourceSdkRequestOptions {
        api_base: params.api_base.clone(),
        api_version: params.api_version.clone(),
        api_key: params.api_key.clone(),
        organization: params.organization.clone(),
        vertex_project: params.vertex_project.clone(),
        vertex_location: params.vertex_location.clone(),
        // an inline service-account object is resolved separately; a path is the shared chain's input
        vertex_credentials: match params.vertex_credentials.as_ref() {
            Some(Json::Text(path)) => Some(SecretString::from(path.clone())),
            _ => None,
        },
    }
}
