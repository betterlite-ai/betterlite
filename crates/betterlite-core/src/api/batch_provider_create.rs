use std::collections::BTreeMap;

use betterlite_config::sections::batch_provider_create::Settings;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::batches::records::{
    BatchEndpoint, BatchObject, CompletionWindow, OutputExpiry,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_provider_create::body::create_request;
use crate::api::batch_provider_create::error::BatchProviderCreateError;
use crate::api::batch_provider_create::provider::build_call;
use crate::api::batch_provider_create::transports::BatchCreateTransport;

mod body;
pub mod error;
mod provider;
mod tests;
mod timeout;
pub mod transports;

pub use timeout::{DEFAULT_TIMEOUT_SECONDS, HttpTimeout, RequestTimeout};

pub const DEFAULT_OPENAI_BASE: &str = "https://api.openai.com/v1";

// The custom_llm_provider literal create_batch declares; the wire spelling is the LiteLLM provider name
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BatchProvider {
    #[default]
    Openai,
    Azure,
    VertexAi,
    Bedrock,
    HostedVllm,
    LitellmProxy,
    Mistral,
    Xai,
}

impl BatchProvider {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Azure => "azure",
            Self::VertexAi => "vertex_ai",
            Self::Bedrock => "bedrock",
            Self::HostedVllm => "hosted_vllm",
            Self::LitellmProxy => "litellm_proxy",
            Self::Mistral => "mistral",
            Self::Xai => "xai",
        }
    }
}

// Vertex takes a service-account file path or the decoded service-account fields
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum VertexCredentials {
    Path(String),
    Fields(BTreeMap<String, String>),
}

// The fields of GenericLiteLLMParams that create_batch reads
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatchOptionalParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: Option<RequestTimeout>,
    pub request_timeout: Option<f64>,
    pub max_retries: Option<u32>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<VertexCredentials>,
    pub custom_endpoint: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchProviderCreateInput {
    pub completion_window: CompletionWindow,
    pub endpoint: BatchEndpoint,
    pub input_file_id: String,
    #[serde(default)]
    pub custom_llm_provider: BatchProvider,
    #[serde(default)]
    pub metadata: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub output_expires_after: Option<OutputExpiry>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub is_async: bool,
    #[serde(default)]
    pub optional_params: BatchOptionalParams,
}

#[derive(Debug, Clone)]
pub enum BatchProviderCreateCall {
    ProviderConfig(ProviderConfigCall),
    Xai(ProviderBatchCall),
    OpenAiCompatible(ProviderBatchCall),
    Azure(ProviderBatchCall),
    VertexAi(ProviderBatchCall),
}

// base_llm_http_handler.create_batch names a provider config instead of a provider
#[derive(Debug, Clone)]
pub struct ProviderConfigCall {
    pub provider: BatchProvider,
    pub model: String,
    pub body: CreateBatchRequest,
    pub headers: BTreeMap<String, String>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub timeout: f64,
    pub is_async: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderBatchCall {
    pub body: CreateBatchRequest,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub api_version: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<VertexCredentials>,
    pub custom_endpoint: Option<bool>,
    pub timeout: f64,
    pub max_retries: Option<u32>,
    pub is_async: bool,
}

#[derive(Debug, Clone)]
pub struct BatchProviderCreateResult {
    pub call: BatchProviderCreateCall,
    pub batch: BatchObject,
}

pub struct BatchProviderCreate<'context, Transport> {
    pub settings: Settings,
    pub resolution: &'context ResolutionContext<'context>,
    pub transport: Transport,
    pub limits: ApiLimits,
}

impl<Transport: BatchCreateTransport> BatchProviderCreate<'_, Transport> {
    pub fn create(
        &self,
        input: &BatchProviderCreateInput,
    ) -> Result<BatchProviderCreateResult, BatchProviderCreateError> {
        let model = input
            .model
            .as_deref()
            .map(|model| resolved_model(model, self.resolution));
        let timeout = timeout::resolve(
            input.optional_params.timeout,
            input.optional_params.request_timeout,
        );
        let body = create_request(input, self.limits)?;
        let call = build_call(input, model.as_deref(), &self.settings, timeout, body)?;
        let batch = self.transport.create(&call)?;
        Ok(BatchProviderCreateResult { call, batch })
    }
}

// get_llm_provider(model, None) normalizes the model and create_batch logs and keeps the raw one when it raises
fn resolved_model(model: &str, resolution: &ResolutionContext<'_>) -> String {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    resolve(&request, resolution).map_or_else(|_error| model.to_owned(), |resolved| resolved.model)
}
