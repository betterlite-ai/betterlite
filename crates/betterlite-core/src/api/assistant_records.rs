pub mod call;
pub mod error;
mod projection;
pub mod service;

mod tests;

use std::collections::BTreeMap;

use betterlite_openai::assistants::records::{AssistantTool, ResponseFormat, ToolResources};
use betterlite_openai::batches::sdk::TimeoutPhases;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

// The client class create_assistants was handed: only isinstance(client, OpenAI) is ever decided from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientClass {
    OpenAi,
    AzureOpenAi,
    AsyncOpenAi,
    AsyncAzureOpenAi,
    Other,
}

// async_create_assistants: any bool is legal, and anything else raises ValueError
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AsyncFlag {
    Bool(bool),
    Other(Value),
}

// GenericLiteLLMParams.timeout: seconds, a number written as text, or the pieces of an httpx.Timeout
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum CreateTimeout {
    Seconds(f64),
    Text(String),
    Phases(TimeoutPhases),
}

// create_assistants' arguments; unknown keys are the kwargs GenericLiteLLMParams accepts and the façade drops
#[derive(Debug, Clone, Deserialize)]
pub struct AssistantCreateInput {
    pub provider: String,
    pub model: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub tools: Option<Vec<AssistantTool>>,
    pub tool_resources: Option<ToolResources>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub response_format: Option<ResponseFormat>,
    pub client: Option<ClientClass>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout: Option<CreateTimeout>,
    pub request_timeout: Option<f64>,
    pub async_create_assistants: Option<AsyncFlag>,
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
    // GenericLiteLLMParams.get("extra_body", {}): absent, explicit null, or a mapping
    pub extra_body: Option<Option<BTreeMap<String, String>>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
