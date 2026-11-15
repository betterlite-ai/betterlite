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
        headers: input
            .extra_headers
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        api_base: input.optional_params.api_base.clone(),
        api_key: input.optional_params.api_key.clone(),
        aws,
        timeout,
        is_async: input.aretrieve_batch,
    }
}

// the shared chains resolved once for the provider branch that reads them
pub(super) struct Resolution<'read> {
    input: &'read BatchProviderReadInput,
    settings: &'read Settings,
    resources: ResourceSdkSettings,
    options: ResourceSdkRequestOptions,
}

impl<'read> Resolution<'read> {
    pub(super) fn new(input: &'read BatchProviderReadInput, settings: &'read Settings) -> Self {
        let params = &input.optional_params;
        // an inline service-account object is resolved separately; a path is the shared chain's input
        let path = match params.vertex_credentials.as_ref() {
            Some(Json::Text(path)) => Some(SecretString::from(path.clone())),
            _ => None,
        };
        let options = ResourceSdkRequestOptions {
            api_base: params.api_base.clone(),
            api_version: params.api_version.clone(),
            api_key: params.api_key.clone(),
            organization: params.organization.clone(),
            vertex_project: params.vertex_project.clone(),
            vertex_location: params.vertex_location.clone(),
            vertex_credentials: path,
        };
        Self {
            input,
            settings,
            resources: ResourceSdkSettings::from(&settings.resources),
            options,
        }
    }

    // OpenAIBatchesAPI.retrieve_batch
    pub(super) fn openai(&self, timeout: Timeout) -> OpenAiBatchReadCall {
        let request = retrieve_request(self.input);
        let api_base = self.resources.openai_base(&self.options);
        let api_key = self.resources.openai_api_key(&self.options).cloned();
        let organization = self.resources.openai_organization(&self.options);
        OpenAiBatchReadCall {
            request,
            api_base,
            api_key,
            organization: organization.map(str::to_owned),
            timeout,
            max_retries: self.input.optional_params.max_retries,
            is_async: self.input.aretrieve_batch,
        }
    }

    // AzureBatchesAPI.retrieve_batch; the client constructor reads the raw kwargs, not the resolved chain
    pub(super) fn azure(&self, timeout: Timeout) -> AzureBatchReadCall {
        let params = &self.input.optional_params;
        AzureBatchReadCall {
            request: retrieve_request(self.input),
            api_base: self.resources.azure_base(&self.options).map(str::to_owned),
            api_key: self.resources.azure_api_key(&self.options).cloned(),
            api_version: self
                .resources
                .azure_version(&self.options)
                .map(str::to_owned),
            timeout,
            max_retries: params.max_retries,
            litellm_params: azure_params(params),
            is_async: self.input.aretrieve_batch,
        }
    }

    // VertexAIBatchPrediction.retrieve_batch
    pub(super) fn vertex(&self, timeout: Timeout) -> Result<VertexBatchReadCall, ReadCallError> {
        let params = &self.input.optional_params;
        let project = self.resources.vertex_project(&self.options);
        let location = self.resources.vertex_location(&self.options);
        Ok(VertexBatchReadCall {
            batch_id: self.input.batch_id.clone(),
            api_base: params.api_base.clone().unwrap_or_default(),
            vertex_project: project.map(str::to_owned),
            vertex_location: location.map(str::to_owned),
            vertex_credentials: self.vertex_credentials()?,
            timeout,
            max_retries: params.max_retries,
            is_async: self.input.aretrieve_batch,
        })
    }

    // LiteLLM declares vertex_credentials as a path string or the inline service-account fields
    fn vertex_credentials(&self) -> Result<Option<VertexReadCredentials>, ReadCallError> {
        match self.input.optional_params.vertex_credentials.as_ref() {
            Some(Json::Object(account)) => Ok(Some(VertexReadCredentials::ServiceAccount(
                Box::new(account.clone()),
            ))),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
                Err(ReadCallError::InvalidVertexCredentials)
            }
            Some(Json::Text(_) | Json::Null) | None => Ok(self
                .resources
                .vertex_credentials(&self.options)
                .cloned()
                .map(VertexReadCredentials::Path)),
        }
    }

    // AnthropicBatchesHandler.retrieve_batch
    pub(super) fn anthropic(&self, timeout: Timeout) -> AnthropicBatchReadCall {
        let params = &self.input.optional_params;
        let shared = &self.settings.resources;
        let bases = [
            params.api_base.as_deref(),
            shared.api_base.as_deref(),
            self.settings.anthropic_base(),
        ];
        // the branch falls back through litellm.azure_key as well, a copy-paste the source keeps
        let keys = [
            params.api_key.as_ref(),
            shared.api_key.as_ref(),
            shared.azure_key.as_ref(),
            self.settings.anthropic_api_key.as_ref(),
        ];
        let api_base = first_text(bases);
        let api_key = first_secret(keys);
        AnthropicBatchReadCall {
            batch_id: self.input.batch_id.clone(),
            api_base,
            api_key,
            timeout,
            max_retries: params.max_retries,
            is_async: self.input.aretrieve_batch,
        }
    }
}

// the Azure client constructor reads the raw kwargs, not the resolved chain
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

fn retrieve_request(input: &BatchProviderReadInput) -> RetrieveBatchRequest {
    RetrieveBatchRequest {
        batch_id: Some(input.batch_id.clone()),
        extra_headers: input.extra_headers.clone(),
        extra_body: input.extra_body.clone(),
        timeout: None,
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
