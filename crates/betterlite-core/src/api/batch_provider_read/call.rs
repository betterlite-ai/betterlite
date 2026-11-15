use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use betterlite_openai::api::batch_records::RetrieveBatchRequest;
use betterlite_openai::batches::sdk::Timeout;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use serde::Deserialize;

use super::{BatchReadProvider, BatchReadTimeoutValue};

// get_litellm_params carries every AWS override into the Bedrock request signature
#[derive(Debug, Clone, Default)]
pub struct AwsCredentials {
    pub region_name: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<SecretString>,
    pub session_token: Option<SecretString>,
    pub session_name: Option<String>,
    pub profile_name: Option<String>,
    pub role_name: Option<String>,
    pub web_identity_token: Option<SecretString>,
    pub sts_endpoint: Option<String>,
    pub external_id: Option<String>,
    pub session_tags: Option<Vec<AwsSessionTag>>,
    pub bedrock_runtime_endpoint: Option<String>,
    pub bedrock_project_id: Option<String>,
}

// GenericLiteLLMParams.aws_session_tags
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AwsSessionTag {
    #[serde(rename = "Key")]
    pub key: String,
    #[serde(rename = "Value")]
    pub value: String,
}

// base_llm_http_handler.retrieve_batch, which the configured Bedrock and Mistral providers reach
#[derive(Debug, Clone)]
pub struct ProviderConfigReadCall {
    pub provider: BatchReadProvider,
    pub model: String,
    pub batch_id: String,
    pub headers: BTreeMap<String, String>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub aws: AwsCredentials,
    pub timeout: Timeout,
    pub is_async: bool,
}

// XAIBatchesHandler.retrieve_batch
#[derive(Debug, Clone)]
pub struct XaiBatchReadCall {
    pub batch_id: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub timeout: Timeout,
    pub is_async: bool,
}

// OpenAIBatchesAPI.retrieve_batch
#[derive(Debug, Clone)]
pub struct OpenAiBatchReadCall {
    pub request: RetrieveBatchRequest,
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub timeout: Timeout,
    pub max_retries: Option<i64>,
    pub is_async: bool,
}

// AzureBatchesAPI.retrieve_batch
#[derive(Debug, Clone)]
pub struct AzureBatchReadCall {
    pub request: RetrieveBatchRequest,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub timeout: Timeout,
    pub max_retries: Option<i64>,
    pub litellm_params: AzureReadParams,
    pub is_async: bool,
}

// the get_litellm_params subset the Azure client constructor reads
#[derive(Debug, Clone, Default)]
pub struct AzureReadParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout: Option<BatchReadTimeoutValue>,
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_scope: Option<String>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
}

// a path or an inline service-account document; both stay out of Debug until the Vertex client opens them
#[derive(Clone)]
pub enum VertexReadCredentials {
    Path(SecretString),
    ServiceAccount(Box<Object>),
}

impl Debug for VertexReadCredentials {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("VertexReadCredentials(<redacted>)")
    }
}

// VertexAIBatchPrediction.retrieve_batch
#[derive(Debug, Clone)]
pub struct VertexBatchReadCall {
    pub batch_id: String,
    pub api_base: String,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<VertexReadCredentials>,
    pub timeout: Timeout,
    pub max_retries: Option<i64>,
    pub is_async: bool,
}

// AnthropicBatchesHandler.retrieve_batch
#[derive(Debug, Clone)]
pub struct AnthropicBatchReadCall {
    pub batch_id: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub timeout: Timeout,
    pub max_retries: Option<i64>,
    pub is_async: bool,
}

// BedrockBatchesHandler._handle_async_invoke_status, whose region defaults to us-east-1
#[derive(Debug, Clone)]
pub struct BedrockAsyncInvokeReadCall {
    pub batch_id: String,
    pub aws: AwsCredentials,
}

// BedrockBatchesHandler._handle_model_invocation_job_status, whose region stays unset
#[derive(Debug, Clone)]
pub struct BedrockJobReadCall {
    pub batch_id: String,
    pub aws: AwsCredentials,
}

// what LiteLLM hands one provider entry point for a retrieve
#[derive(Debug, Clone)]
pub enum BatchProviderReadCall {
    ProviderConfig(Box<ProviderConfigReadCall>),
    Xai(Box<XaiBatchReadCall>),
    OpenAiCompatible(Box<OpenAiBatchReadCall>),
    Azure(Box<AzureBatchReadCall>),
    VertexAi(Box<VertexBatchReadCall>),
    Anthropic(Box<AnthropicBatchReadCall>),
    BedrockAsyncInvoke(Box<BedrockAsyncInvokeReadCall>),
    BedrockModelInvocationJob(Box<BedrockJobReadCall>),
}
