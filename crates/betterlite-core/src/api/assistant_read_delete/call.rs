use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;

use super::request::{AssistantOperation, SuppliedClient};

// everything the Azure provider method and the client it initializes receive
#[derive(Debug, Clone)]
pub struct AzureAssistantCall {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub timeout: f64,
    pub max_retries: Option<i64>,
    // get_litellm_params: the kwargs LiteLLM forwards to the Azure client initializer
    pub params: LiteLlmParams,
    // extra_body after the azure_ad_token key was popped, so the rest is never forwarded either
    pub extra_body: IndexMap<String, Json>,
}

#[derive(Debug, Clone)]
pub enum AssistantProviderCall {
    OpenAi(Box<ClientOptions>),
    Azure(Box<AzureAssistantCall>),
}

// One get/aget_assistants or delete/adelete_assistant call after the facade resolved its arguments.
#[derive(Debug, Clone)]
pub struct AssistantCall {
    pub operation: AssistantOperation,
    pub provider: AssistantProviderCall,
    pub assistant_id: Option<String>,
    // the client kind the facade's flag selected; a supplied client keeps its own kind
    pub kind: ClientKind,
    pub supplied: Option<SuppliedClient>,
    // the timeout every branch resolved before it reached the provider, structured phases included
    pub timeout: FineTuningTimeout,
}
