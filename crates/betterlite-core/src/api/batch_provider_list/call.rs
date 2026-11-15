use betterlite_openai::batches::sdk::Timeout;
use secrecy::SecretString;

use super::BatchListTimeoutValue;
use crate::genai_setup::params::VertexCredentials;

// the list window every provider's call carries
#[derive(Debug, Clone)]
pub struct BatchListWindow {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub timeout: Timeout,
}

// the keyword arguments LiteLLM passes to XAIBatchesHandler.list_batches
#[derive(Debug, Clone)]
pub struct XaiBatchListCall {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

// the keyword arguments LiteLLM passes to OpenAIBatchesAPI.list_batches
#[derive(Debug, Clone)]
pub struct OpenAiBatchListCall {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub max_retries: Option<i64>,
}

// the keyword arguments LiteLLM passes to VertexAIBatchPrediction.list_batches
#[derive(Debug, Clone)]
pub struct VertexBatchListCall {
    pub api_base: String,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<VertexCredentials>,
    pub max_retries: Option<i64>,
}

// the keyword arguments LiteLLM passes to AzureBatchesAPI.list_batches
#[derive(Debug, Clone)]
pub struct AzureBatchListCall {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub max_retries: Option<i64>,
    pub litellm_params: AzureBatchListParams,
}

// the subset of get_litellm_params the Azure client constructor reads
#[derive(Debug, Clone)]
pub struct AzureBatchListParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout: Option<BatchListTimeoutValue>,
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_scope: Option<String>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
}

// what LiteLLM passes only to this provider's list call
#[derive(Debug, Clone)]
pub enum BatchListProviderCall {
    Azure(Box<AzureBatchListCall>),
    OpenAiCompatible(Box<OpenAiBatchListCall>),
    VertexAi(Box<VertexBatchListCall>),
    Xai(Box<XaiBatchListCall>),
}

// the shared list window next to the record of the provider it goes to
#[derive(Debug, Clone)]
pub struct BatchProviderListCall {
    pub window: BatchListWindow,
    pub provider: BatchListProviderCall,
}
