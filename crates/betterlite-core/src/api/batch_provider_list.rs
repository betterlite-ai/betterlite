pub mod call;
pub mod error;
mod timeout;

#[path = "batch_provider_list/tests.rs"]
mod tests;

use betterlite_config::sections::batch_provider_list::Settings as ConfiguredSettings;
use betterlite_openai::batches::sdk::TimeoutPhases;
use secrecy::SecretString;
use serde::Deserialize;

use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};
use call::{
    AzureBatchListCall, AzureBatchListParams, BatchProviderListCall, OpenAiBatchListCall,
    VertexBatchListCall, XaiBatchListCall,
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
        match name.as_str() {
            "azure" => Self::Azure,
            "hosted_vllm" => Self::HostedVllm,
            "litellm_proxy" => Self::LitellmProxy,
            "openai" => Self::OpenAi,
            "vertex_ai" => Self::VertexAi,
            "xai" => Self::Xai,
            _ => Self::Other(name),
        }
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
    pub vertex_credentials: Option<SecretString>,
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

impl BatchProviderList {
    pub fn run(
        &self,
        input: &BatchProviderListInput,
    ) -> Result<BatchProviderListResult, BatchProviderListError> {
        let request = ResourceSdkRequestOptions {
            api_base: input.api_base.clone(),
            api_version: input.api_version.clone(),
            api_key: input.api_key.clone(),
            organization: input.organization.clone(),
            vertex_project: input.vertex_project.clone(),
            vertex_location: input.vertex_location.clone(),
            vertex_credentials: input.vertex_credentials.clone(),
        };
        let call = match &input.provider {
            BatchListProvider::Xai => BatchProviderListCall::Xai(Box::new(self.xai_call(input)?)),
            BatchListProvider::Azure => {
                BatchProviderListCall::Azure(Box::new(self.azure_call(input, &request)?))
            }
            BatchListProvider::VertexAi => {
                BatchProviderListCall::VertexAi(Box::new(self.vertex_call(input, &request)?))
            }
            BatchListProvider::OpenAi
            | BatchListProvider::HostedVllm
            | BatchListProvider::LitellmProxy => BatchProviderListCall::OpenAiCompatible(Box::new(
                self.openai_call(input, &request)?,
            )),
            BatchListProvider::Other(provider) => {
                return Err(BatchProviderListError::UnsupportedProvider {
                    provider: provider.clone(),
                });
            }
        };
        Ok(BatchProviderListResult {
            call,
            is_async: input.alist_batches,
        })
    }

    fn xai_call(
        &self,
        input: &BatchProviderListInput,
    ) -> Result<XaiBatchListCall, BatchProviderListError> {
        Ok(XaiBatchListCall {
            after: input.after.clone(),
            limit: input.limit,
            api_base: input.api_base.clone(),
            api_key: input.api_key.clone(),
            timeout: resolve_timeout(input, false)?,
        })
    }

    fn openai_call(
        &self,
        input: &BatchProviderListInput,
        request: &ResourceSdkRequestOptions,
    ) -> Result<OpenAiBatchListCall, BatchProviderListError> {
        Ok(OpenAiBatchListCall {
            after: input.after.clone(),
            limit: input.limit,
            api_base: self.settings.openai_base(request),
            api_key: self.settings.openai_api_key(request).cloned(),
            organization: self
                .settings
                .openai_organization(request)
                .map(str::to_owned),
            timeout: resolve_timeout(input, input.provider.supports_httpx_timeout())?,
            max_retries: input.max_retries,
        })
    }

    fn azure_call(
        &self,
        input: &BatchProviderListInput,
        request: &ResourceSdkRequestOptions,
    ) -> Result<AzureBatchListCall, BatchProviderListError> {
        Ok(AzureBatchListCall {
            api_base: self.settings.azure_base(request).map(str::to_owned),
            api_key: self.settings.azure_api_key(request).cloned(),
            api_version: self.settings.azure_version(request).map(str::to_owned),
            timeout: resolve_timeout(input, true)?,
            max_retries: input.max_retries,
            litellm_params: AzureBatchListParams {
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
            },
        })
    }

    fn vertex_call(
        &self,
        input: &BatchProviderListInput,
        request: &ResourceSdkRequestOptions,
    ) -> Result<VertexBatchListCall, BatchProviderListError> {
        Ok(VertexBatchListCall {
            after: input.after.clone(),
            limit: input.limit,
            api_base: input.api_base.clone().unwrap_or_default(),
            vertex_project: self.settings.vertex_project(request).map(str::to_owned),
            vertex_location: self.settings.vertex_location(request).map(str::to_owned),
            vertex_credentials: self.settings.vertex_credentials(request).cloned(),
            timeout: resolve_timeout(input, false)?,
            max_retries: input.max_retries,
        })
    }
}
