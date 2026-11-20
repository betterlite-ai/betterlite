use betterlite_azure::azure_urls::AuthParams;
use betterlite_openai::assistants::records::{AssistantCreate, AssistantObject};
use secrecy::SecretString;

use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;

use super::ClientClass;
use super::error::AssistantCreateFailure;

// create_assistants' arguments for the openai provider, before the SDK client is built from them
#[derive(Debug, Clone)]
pub struct OpenAiCreateCall {
    pub create: AssistantCreate,
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub timeout: FineTuningTimeout,
    pub max_retries: Option<i64>,
    // the caller's client class, recorded because a supplied client replaces every option above
    pub client: Option<ClientClass>,
    pub is_async: bool,
}

// get_litellm_params' subset that initialize_azure_sdk_client reads; timeout stays the raw kwarg
#[derive(Debug, Clone, Default)]
pub struct AzureClientParams {
    pub azure_ad_token: Option<SecretString>,
    pub auth: AuthParams,
    pub max_retries: Option<i64>,
    pub timeout: Option<f64>,
}

// create_assistants' arguments for the azure provider; token, timeout and retries are the facade's own
#[derive(Debug, Clone)]
pub struct AzureCreateCall {
    pub create: AssistantCreate,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub azure_ad_token: Option<SecretString>,
    pub timeout: FineTuningTimeout,
    pub max_retries: Option<i64>,
    pub client: Option<ClientClass>,
    pub is_async: bool,
    pub litellm_params: AzureClientParams,
}

#[derive(Debug, Clone)]
pub enum AssistantCreateCall {
    OpenAi(Box<OpenAiCreateCall>),
    Azure(Box<AzureCreateCall>),
}

// The provider answer next to the arguments that produced it, so the choice of client is observable.
#[derive(Debug, Clone)]
pub struct AssistantCreateResult {
    pub call: AssistantCreateCall,
    pub assistant: AssistantObject,
}

// What both create_assistants façades answer with, so their signatures name the façade's own failure.
pub type AssistantCreateOutcome = Result<AssistantCreateResult, Box<AssistantCreateFailure>>;
