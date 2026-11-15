use std::collections::BTreeMap;

use betterlite_config::sections::batch_provider_read::Settings;
use betterlite_openai::api::batch_records::RetrieveBatchRequest;
use betterlite_openai::batches::sdk::Timeout;
use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};

use super::call::{
    AnthropicBatchReadCall, AwsCredentials, AzureBatchReadCall, AzureReadParams,
    OpenAiBatchReadCall, ProviderConfigReadCall, VertexBatchReadCall, XaiBatchReadCall,
};
use super::error::ReadCallError;
use super::{BatchProviderReadInput, BatchReadOptionalParams};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};
use crate::genai_setup::params::VertexCredentials;

// base_llm_http_handler.retrieve_batch, which the configured Bedrock and Mistral providers reach
pub(super) fn provider_config(
    input: &BatchProviderReadInput,
    model: &str,
    timeout: Timeout,
) -> ProviderConfigReadCall {
    ProviderConfigReadCall {
        provider: input.provider.clone(),
        model: model.to_owned(),
        batch_id: input.batch_id.clone(),
        headers: input
            .extra_headers
            .as_ref()
            .map_or_else(BTreeMap::new, |headers| {
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            }),
        api_base: input.optional_params.api_base.clone(),
        api_key: input.optional_params.api_key.clone(),
        aws: aws_credentials(&input.optional_params),
        timeout,
        is_async: input.aretrieve_batch,
    }
}

// xai_batches_instance.retrieve_batch, which reads the request kwargs alone
pub(super) fn xai(input: &BatchProviderReadInput, timeout: Timeout) -> XaiBatchReadCall {
    XaiBatchReadCall {
        batch_id: input.batch_id.clone(),
        api_base: input.optional_params.api_base.clone(),
        api_key: input.optional_params.api_key.clone(),
        timeout,
        is_async: input.aretrieve_batch,
    }
}

// OpenAIBatchesAPI.retrieve_batch
pub(super) fn openai(
    input: &BatchProviderReadInput,
    request: &RetrieveBatchRequest,
    settings: &Settings,
    timeout: Timeout,
) -> OpenAiBatchReadCall {
    let resources = resource_settings(settings);
    let options = request_options(&input.optional_params);
    OpenAiBatchReadCall {
        request: request.clone(),
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
    request: &RetrieveBatchRequest,
    settings: &Settings,
    timeout: Timeout,
) -> AzureBatchReadCall {
    let resources = resource_settings(settings);
    let options = request_options(&input.optional_params);
    AzureBatchReadCall {
        request: request.clone(),
        api_base: resources.azure_base(&options).map(str::to_owned),
        api_key: resources.azure_api_key(&options).cloned(),
        api_version: resources.azure_version(&options).map(str::to_owned),
        timeout,
        max_retries: input.optional_params.max_retries,
        litellm_params: azure_params(&input.optional_params),
        is_async: input.aretrieve_batch,
    }
}

// get_litellm_params keeps these kwargs for the Azure client constructor, unresolved
fn azure_params(params: &BatchReadOptionalParams) -> AzureReadParams {
    AzureReadParams {
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
    }
}

// VertexAIBatchPrediction.retrieve_batch
pub(super) fn vertex(
    input: &BatchProviderReadInput,
    settings: &Settings,
    timeout: Timeout,
) -> Result<VertexBatchReadCall, ReadCallError> {
    let resources = resource_settings(settings);
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
) -> Result<Option<VertexCredentials>, ReadCallError> {
    match params.vertex_credentials.as_ref() {
        Some(Json::Object(account)) => Ok(Some(VertexCredentials::ServiceAccount(account.clone()))),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(ReadCallError::InvalidVertexCredentials)
        }
        Some(Json::Text(_) | Json::Null) | None => Ok(resources
            .vertex_credentials(options)
            .map(|text| VertexCredentials::Path(text.expose_secret().to_owned()))),
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

// the litellm_settings keys retrieve_batch falls back through, under the names the config loads them as
fn resource_settings(settings: &Settings) -> ResourceSdkSettings {
    let resources = &settings.resources;
    ResourceSdkSettings {
        api_base: resources.api_base.clone(),
        api_version: resources.api_version.clone(),
        api_key: resources.api_key.clone(),
        openai_key: resources.openai_key.clone(),
        azure_key: resources.azure_key.clone(),
        azure_ad_token: resources.azure_ad_token.clone(),
        organization: resources.organization.clone(),
        vertex_project: resources.vertex_project.clone(),
        vertex_location: resources.vertex_location.clone(),
        openai_base_url: resources.openai_base_url.clone(),
        openai_api_base: resources.openai_api_base.clone(),
        openai_api_key: resources.openai_api_key.clone(),
        openai_organization: resources.openai_organization.clone(),
        azure_api_base: resources.azure_api_base.clone(),
        azure_api_version: resources.azure_api_version.clone(),
        azure_openai_api_key: resources.azure_openai_api_key.clone(),
        azure_api_key: resources.azure_api_key.clone(),
        vertexai_project: resources.vertexai_project.clone(),
        vertexai_location: resources.vertexai_location.clone(),
        vertexai_credentials: resources.vertexai_credentials.clone(),
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

pub(super) fn aws_credentials(params: &BatchReadOptionalParams) -> AwsCredentials {
    AwsCredentials {
        region_name: params.aws_region_name.clone(),
        access_key_id: params.aws_access_key_id.clone(),
        secret_access_key: params.aws_secret_access_key.clone(),
        session_token: params.aws_session_token.clone(),
    }
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
