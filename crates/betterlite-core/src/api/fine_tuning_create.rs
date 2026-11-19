use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::{FineTuningTimeout, TimeoutError};

pub mod connection;
pub mod service;
mod tests;

use connection::CreateParams;

pub const UNSUPPORTED_RESPONSE_BODY: &str = "Unsupported provider";
pub const UNSUPPORTED_REQUEST_METHOD: &str = "create_thread";
pub const UNSUPPORTED_REQUEST_URL: &str = "https://github.com/BerriAI/litellm";
pub const UNSUPPORTED_MODEL: &str = "n/a";

// the source declares a three-field record that allows extra keys, which is B518's extensible record
pub type OpenAIFineTuningHyperparameters = betterlite_openai::fine_tuning::job::Hyperparameters;

fn default_provider() -> String {
    "openai".to_owned()
}

// create_fine_tuning_job's named arguments plus the kwargs generic params reads
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateFineTuningJobCall {
    pub model: String,
    pub training_file: String,
    #[serde(default)]
    pub hyperparameters: Option<Extra>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub validation_file: Option<String>,
    // the declared list[str] is looser than the SDK record, so an entry the record refuses fails here
    #[serde(default)]
    pub integrations: Option<Vec<Json>>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default = "default_provider")]
    pub custom_llm_provider: String,
    // the public arguments reach no provider: OpenAI ignores them and Azure overwrites extra_body
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<Extra>,
    #[serde(default)]
    pub params: CreateParams,
    #[serde(default)]
    pub is_async: bool,
}

// the BadRequestError an unsupported provider raises: openai's APIError leaves type, param and code null,
// and the source attaches an httpx.Response that stays on the error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedProviderError {
    pub status: u16,
    pub error_type: Option<&'static str>,
    pub param: Option<&'static str>,
    pub code: Option<&'static str>,
    pub body: &'static str,
    pub method: &'static str,
    pub url: &'static str,
}

#[derive(Debug, Error)]
pub enum FineTuningCreateError<DispatchError = Infallible>
where
    DispatchError: Error + 'static,
{
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("the fine-tuning job data is not a valid record")]
    Data {
        #[source]
        source: serde_json::Error,
    },
    #[error("LiteLLM doesn't support {provider} for 'create_batch'. Only 'openai' is supported.")]
    Unsupported { provider: String },
    #[error(transparent)]
    Dispatch(DispatchError),
}

impl<DispatchError> FineTuningCreateError<DispatchError>
where
    DispatchError: Error + 'static,
{
    pub const fn unsupported_provider(&self) -> Option<UnsupportedProviderError> {
        match self {
            Self::Unsupported { .. } => Some(UnsupportedProviderError {
                status: 400,
                error_type: None,
                param: None,
                code: None,
                body: UNSUPPORTED_RESPONSE_BODY,
                method: UNSUPPORTED_REQUEST_METHOD,
                url: UNSUPPORTED_REQUEST_URL,
            }),
            Self::Timeout(_) | Self::Data { .. } | Self::Dispatch(_) => None,
        }
    }
}

// every provider branch reads these after the shared timeout and hyperparameter preparation
#[derive(Debug, Clone)]
pub struct OpenAiCreateCall {
    pub is_async: bool,
    pub data: FineTuneCreate,
    // the openai branch always has a base: the call's, the settings' or the default
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: FineTuningTimeout,
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AzureCreateCall {
    pub is_async: bool,
    pub data: FineTuneCreate,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: FineTuningTimeout,
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct VertexCreateCall {
    pub is_async: bool,
    pub data: FineTuneCreate,
    // the caller's map after the Azure-only extraction, which is where adapter_size is read
    pub original_hyperparameters: Extra,
    // passed through as the source does; B1384 builds the regional URL without it
    pub api_base: String,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<SecretString>,
    pub timeout: FineTuningTimeout,
}

// create_fine_tuning_job's three providers; the provider's wire name decides which one a call takes
#[derive(Debug, Clone)]
pub enum FineTuningCreateTarget {
    OpenAi(OpenAiCreateCall),
    Azure(AzureCreateCall),
    Vertex(VertexCreateCall),
}

pub trait FineTuningCreateDispatch: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn create(
        &self,
        target: FineTuningCreateTarget,
    ) -> impl Future<Output = Result<FineTuneJob, Self::Error>> + Send;
}
