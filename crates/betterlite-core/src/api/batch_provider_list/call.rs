use betterlite_openai::batches::sdk::Timeout;
use secrecy::SecretString;

use super::BatchListTimeoutValue;

// the keyword arguments LiteLLM passes to XAIBatchesHandler.list_batches
#[derive(Debug, Clone)]
pub struct XaiBatchListCall {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub timeout: Timeout,
}

// the keyword arguments LiteLLM passes to OpenAIBatchesAPI.list_batches
#[derive(Debug, Clone)]
pub struct OpenAiBatchListCall {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub timeout: Timeout,
    pub max_retries: Option<i64>,
}

// the keyword arguments LiteLLM passes to VertexAIBatchPrediction.list_batches
#[derive(Debug, Clone)]
pub struct VertexBatchListCall {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub api_base: String,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<SecretString>,
    pub timeout: Timeout,
    pub max_retries: Option<i64>,
}

// the keyword arguments LiteLLM passes to AzureBatchesAPI.list_batches
#[derive(Debug, Clone)]
pub struct AzureBatchListCall {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub timeout: Timeout,
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

#[derive(Debug, Clone)]
pub enum BatchProviderListCall {
    Azure(Box<AzureBatchListCall>),
    OpenAiCompatible(Box<OpenAiBatchListCall>),
    VertexAi(Box<VertexBatchListCall>),
    Xai(Box<XaiBatchListCall>),
}
