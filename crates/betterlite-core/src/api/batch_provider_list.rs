pub mod call;
pub mod error;
mod timeout;

#[path = "batch_provider_list/tests.rs"]
mod tests;

use betterlite_config::sections::batch_provider_list::Settings as ConfiguredSettings;
use betterlite_openai::batches::sdk::TimeoutPhases;
use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};
use crate::genai_setup::params::VertexCredentials;
use call::{
    AzureBatchListCall, AzureBatchListParams, BatchListProviderCall, BatchListWindow,
    BatchProviderListCall, OpenAiBatchListCall, VertexBatchListCall, XaiBatchListCall,
};
use error::BatchProviderListError;
use timeout::resolve_timeout;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum BatchListProvider {
    Azure,
    HostedVllm,
    LitellmProxy,
    OpenAi,
    VertexAi,
    Xai,
    Other(String),
}

impl From<String> for BatchListProvider {
    fn from(name: String) -> Self {
        let known = [
            Self::Azure,
            Self::HostedVllm,
            Self::LitellmProxy,
            Self::OpenAi,
            Self::VertexAi,
            Self::Xai,
        ];
        let matched = known
            .into_iter()
            .find(|provider| provider.name() == name.as_str());
        matched.unwrap_or(Self::Other(name))
    }
}

impl BatchListProvider {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Azure => "azure",
            Self::HostedVllm => "hosted_vllm",
            Self::LitellmProxy => "litellm_proxy",
            Self::OpenAi => "openai",
            Self::VertexAi => "vertex_ai",
            Self::Xai => "xai",
            Self::Other(name) => name,
        }
    }

    // LiteLLM keeps an httpx.Timeout only for openai and azure, and reads it for every other provider
    #[must_use]
    pub const fn supports_httpx_timeout(&self) -> bool {
        match self {
            Self::Azure | Self::OpenAi => true,
            Self::HostedVllm | Self::LitellmProxy | Self::VertexAi | Self::Xai | Self::Other(_) => {
                false
            }
        }
    }
}

const fn default_provider() -> BatchListProvider {
    BatchListProvider::OpenAi
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchProviderListInput {
    #[serde(default = "default_provider")]
    pub provider: BatchListProvider,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub alist_batches: bool,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub timeout: Option<BatchListTimeoutValue>,
    #[serde(default)]
    pub request_timeout: Option<f64>,
    #[serde(default)]
    pub max_retries: Option<i64>,
    #[serde(default)]
    pub vertex_project: Option<String>,
    #[serde(default)]
    pub vertex_location: Option<String>,
    #[serde(default)]
    pub vertex_credentials: Option<Json>,
    #[serde(default)]
    pub azure_ad_token: Option<SecretString>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<SecretString>,
    #[serde(default)]
    pub azure_scope: Option<String>,
    #[serde(default)]
    pub azure_username: Option<String>,
    #[serde(default)]
    pub azure_password: Option<SecretString>,
}

// the declared type is float | str | httpx.Timeout | None, and only a str is converted
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BatchListTimeoutValue {
    Seconds(f64),
    Text(String),
    Phases(TimeoutPhases),
}

#[derive(Debug, Clone)]
pub struct BatchProviderListResult {
    pub call: BatchProviderListCall,
    pub is_async: bool,
}

#[derive(Debug, Clone)]
pub struct BatchProviderList {
    pub settings: ResourceSdkSettings,
}

impl From<ConfiguredSettings> for ResourceSdkSettings {
    fn from(settings: ConfiguredSettings) -> Self {
        Self {
            api_base: settings.api_base,
            api_version: settings.api_version,
            api_key: settings.api_key,
            openai_key: settings.openai_key,
            azure_key: settings.azure_key,
            azure_ad_token: settings.azure_ad_token,
            organization: settings.organization,
            vertex_project: settings.vertex_project,
            vertex_location: settings.vertex_location,
            openai_base_url: settings.openai_base_url,
            openai_api_base: settings.openai_api_base,
            openai_api_key: settings.openai_api_key,
            openai_organization: settings.openai_organization,
            azure_api_base: settings.azure_api_base,
            azure_api_version: settings.azure_api_version,
            azure_openai_api_key: settings.azure_openai_api_key,
            azure_api_key: settings.azure_api_key,
            vertexai_project: settings.vertexai_project,
            vertexai_location: settings.vertexai_location,
            vertexai_credentials: settings.vertexai_credentials,
        }
    }
}

fn request_options(input: &BatchProviderListInput) -> ResourceSdkRequestOptions {
    ResourceSdkRequestOptions {
        api_base: input.api_base.clone(),
        api_version: input.api_version.clone(),
        api_key: input.api_key.clone(),
        organization: input.organization.clone(),
        vertex_project: input.vertex_project.clone(),
        vertex_location: input.vertex_location.clone(),
        vertex_credentials: match input.vertex_credentials.as_ref() {
            Some(Json::Text(path)) => Some(SecretString::from(path.clone())),
            _ => None,
        },
    }
}

// LiteLLM declares vertex_credentials as a path string or an inline service-account object
fn resolve_vertex_credentials(
    settings: &ResourceSdkSettings,
    request: &ResourceSdkRequestOptions,
    input: &BatchProviderListInput,
) -> Result<Option<VertexCredentials>, BatchProviderListError> {
    let credentials = input.vertex_credentials.as_ref();
    if let Some(Json::Object(account)) = credentials {
        return Ok(Some(VertexCredentials::ServiceAccount(account.clone())));
    }
    if let Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) = credentials {
        return Err(BatchProviderListError::InvalidVertexCredentials);
    }
    // a path string is already on the request options, so the shared chain resolves it
    Ok(settings
        .vertex_credentials(request)
        .map(|text| VertexCredentials::Path(text.expose_secret().to_owned())))
}

fn azure_params(input: &BatchProviderListInput) -> AzureBatchListParams {
    AzureBatchListParams {
        api_key: input.api_key.clone(),
        api_base: input.api_base.clone(),
        api_version: input.api_version.clone(),
        max_retries: input.max_retries,
        timeout: input.timeout.clone(),
        azure_ad_token: input.azure_ad_token.clone(),
        tenant_id: input.tenant_id.clone(),
        client_id: input.client_id.clone(),
        client_secret: input.client_secret.clone(),
        azure_scope: input.azure_scope.clone(),
        azure_username: input.azure_username.clone(),
        azure_password: input.azure_password.clone(),
    }
}

impl BatchProviderList {
    pub fn run(
        &self,
        input: &BatchProviderListInput,
    ) -> Result<BatchProviderListResult, BatchProviderListError> {
        let request = request_options(input);
        Ok(BatchProviderListResult {
            call: BatchProviderListCall {
                window: BatchListWindow {
                    after: input.after.clone(),
                    limit: input.limit,
                    timeout: resolve_timeout(input, input.provider.supports_httpx_timeout())?,
                },
                provider: self.provider_call(input, &request)?,
            },
            is_async: input.alist_batches,
        })
    }

    fn provider_call(
        &self,
        input: &BatchProviderListInput,
        request: &ResourceSdkRequestOptions,
    ) -> Result<BatchListProviderCall, BatchProviderListError> {
        let provider = match &input.provider {
            BatchListProvider::Xai => BatchListProviderCall::Xai(Box::new(XaiBatchListCall {
                api_base: input.api_base.clone(),
                api_key: input.api_key.clone(),
            })),
            BatchListProvider::Azure => {
                BatchListProviderCall::Azure(Box::new(AzureBatchListCall {
                    api_base: self.settings.azure_base(request).map(str::to_owned),
                    api_key: self.settings.azure_api_key(request).cloned(),
                    api_version: self.settings.azure_version(request).map(str::to_owned),
                    max_retries: input.max_retries,
                    litellm_params: azure_params(input),
                }))
            }
            BatchListProvider::VertexAi => {
                BatchListProviderCall::VertexAi(Box::new(VertexBatchListCall {
                    api_base: input.api_base.clone().unwrap_or_default(),
                    vertex_project: self.settings.vertex_project(request).map(str::to_owned),
                    vertex_location: self.settings.vertex_location(request).map(str::to_owned),
                    vertex_credentials: resolve_vertex_credentials(&self.settings, request, input)?,
                    max_retries: input.max_retries,
                }))
            }
            BatchListProvider::OpenAi
            | BatchListProvider::HostedVllm
            | BatchListProvider::LitellmProxy => self.openai_call(request, input.max_retries),
            BatchListProvider::Other(provider) => {
                return Err(BatchProviderListError::UnsupportedProvider {
                    provider: provider.clone(),
                });
            }
        };
        Ok(provider)
    }

    fn openai_call(
        &self,
        request: &ResourceSdkRequestOptions,
        max_retries: Option<i64>,
    ) -> BatchListProviderCall {
        BatchListProviderCall::OpenAiCompatible(Box::new(OpenAiBatchListCall {
            api_base: self.settings.openai_base(request),
            api_key: self.settings.openai_api_key(request).cloned(),
            organization: self
                .settings
                .openai_organization(request)
                .map(str::to_owned),
            max_retries,
        }))
    }
}
