pub mod call;
pub mod error;
mod provider;
mod resolution;
mod timeout;

#[path = "batch_provider_read/tests.rs"]
mod tests;

use std::error::Error;
use std::future::Future;

use betterlite_config::sections::batch_provider_read::Settings;
use betterlite_openai::api::batch_records::RetrieveBatchRequest;
use betterlite_openai::batches::records::BatchObject;
use betterlite_openai::batches::sdk::TimeoutPhases;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

use crate::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, ItemsBoundary, JsonBoundary,
};
use call::BatchProviderReadCall;
use error::BatchProviderReadError;

// retrieve_batch's declared literal names plus any other string, which answers 400
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum BatchReadProvider {
    Anthropic,
    Azure,
    Bedrock,
    HostedVllm,
    LitellmProxy,
    Mistral,
    OpenAi,
    VertexAi,
    Xai,
    Other(String),
}

impl From<String> for BatchReadProvider {
    fn from(name: String) -> Self {
        let known = [
            Self::Anthropic,
            Self::Azure,
            Self::Bedrock,
            Self::HostedVllm,
            Self::LitellmProxy,
            Self::Mistral,
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

impl BatchReadProvider {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Azure => "azure",
            Self::Bedrock => "bedrock",
            Self::HostedVllm => "hosted_vllm",
            Self::LitellmProxy => "litellm_proxy",
            Self::Mistral => "mistral",
            Self::OpenAi => "openai",
            Self::VertexAi => "vertex_ai",
            Self::Xai => "xai",
            Self::Other(name) => name,
        }
    }

    // LiteLLM keeps an httpx.Timeout for openai, azure and bedrock and reads it for every other provider
    #[must_use]
    pub const fn supports_httpx_timeout(&self) -> bool {
        matches!(self, Self::Azure | Self::OpenAi | Self::Bedrock)
    }
}

const fn default_provider() -> BatchReadProvider {
    BatchReadProvider::OpenAi
}

// retrieve_batch's arguments and the GenericLiteLLMParams kwargs it reads them from
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchProviderReadInput {
    pub batch_id: String,
    #[serde(default = "default_provider")]
    pub provider: BatchReadProvider,
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub model: Option<String>,
    // aretrieve_batch sets this before retrieve_batch turns it into the provider's is_async flag
    #[serde(default)]
    pub aretrieve_batch: bool,
    #[serde(default)]
    pub optional_params: BatchReadOptionalParams,
}

// GenericLiteLLMParams.timeout: seconds, a number written as text, or the pieces of an httpx.Timeout
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum BatchReadTimeoutValue {
    Seconds(f64),
    Text(String),
    Phases(TimeoutPhases),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatchReadOptionalParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout: Option<BatchReadTimeoutValue>,
    pub request_timeout: Option<f64>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<Json>,
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_scope: Option<String>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub aws_region_name: Option<String>,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct BatchProviderReadResult {
    pub call: BatchProviderReadCall,
    pub batch: BatchObject,
}

pub trait BatchReadDispatch: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn retrieve(
        &self,
        call: &BatchProviderReadCall,
    ) -> impl Future<Output = Result<BatchObject, Self::Error>> + Send;
}

pub struct BatchProviderRead<Dispatch> {
    pub dispatch: Dispatch,
    pub limits: ApiLimits,
    pub settings: Settings,
}

impl<Dispatch> BatchProviderRead<Dispatch>
where
    Dispatch: BatchReadDispatch,
    Dispatch::Error: Error + Send + Sync + 'static,
{
    pub async fn run(
        &self,
        input: &BatchProviderReadInput,
    ) -> Result<BatchProviderReadResult, BatchProviderReadError<Dispatch::Error>> {
        check_read_limits(&self.limits, input)?;
        let timeout = timeout::resolve_timeout(input)?;
        let call = match provider::bedrock_arn_call(input) {
            Some(call) => call,
            None => provider::call(input, &retrieve_request(input), timeout, &self.settings)?,
        };
        let batch = self
            .dispatch
            .retrieve(&call)
            .await
            .map_err(BatchProviderReadError::Dispatch)?;
        Ok(BatchProviderReadResult { call, batch })
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

// the boundary check every retrieve shares before it reads its provider
fn check_read_limits(
    limits: &ApiLimits,
    input: &BatchProviderReadInput,
) -> Result<(), ApiLimitError> {
    let headers = input.extra_headers.as_ref().map_or(0, IndexMap::len);
    let body = input.extra_body.as_ref().map_or(0, IndexMap::len);
    limits.check(ApiBoundariesInput::Items(ItemsBoundary {
        known_count: headers.saturating_add(body),
    }))?;
    let extra_bytes = input
        .extra_headers
        .as_ref()
        .map_or(0, text_bytes)
        .saturating_add(input.extra_body.as_ref().map_or(0, text_bytes));
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: input.batch_id.len().saturating_add(extra_bytes),
    }))?;
    Ok(())
}

fn text_bytes(entries: &IndexMap<String, String>) -> usize {
    entries
        .iter()
        .map(|(name, value)| name.len().saturating_add(value.len()))
        .fold(0, usize::saturating_add)
}
