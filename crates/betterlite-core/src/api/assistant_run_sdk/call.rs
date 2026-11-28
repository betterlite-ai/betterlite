use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::thread_runs::stream::RunEventHandler;
use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;

use crate::api::resource_sdk_settings::timeout::{StructuredTimeout, TimeoutValue};

// both switches test `is True`: only a JSON true selects the async or streaming branch
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RunFlag {
    Bool(bool),
    Number(f64),
    Text(String),
}

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RunTimeout {
    Plain(TimeoutValue),
    Structured(StructuredTimeout),
}

// the kwargs the run wrappers read through GenericLiteLLMParams and get_litellm_params
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
    pub timeout: Option<RunTimeout>,
    pub request_timeout: Option<f64>,
    pub max_retries: Option<i64>,
    // None: the key is absent and the default {} pops nothing; Some(Null): explicit null reads the env token
    pub extra_body: Option<JsonValue>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunClientClass {
    #[default]
    OpenAi,
    AsyncOpenAi,
    Azure,
    AsyncAzure,
}

// the constructor arguments a supplied SDK client was built with; a supplied client wins over the facade
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunClient {
    pub class: RunClientClass,
    pub api_key: Option<SecretString>,
    pub base_url: Option<String>,
    pub api_version: Option<String>,
    pub azure_ad_token: Option<SecretString>,
    pub timeout: Option<f64>,
    pub max_retries: Option<i64>,
    pub organization: Option<String>,
}

impl RunClient {
    #[must_use]
    pub const fn kind(&self) -> ClientKind {
        match self.class {
            RunClientClass::OpenAi | RunClientClass::Azure => ClientKind::Sync,
            RunClientClass::AsyncOpenAi | RunClientClass::AsyncAzure => ClientKind::Asynchronous,
        }
    }
}

// run_thread's positional arguments: the three required ones have no default, the rest default to null
#[derive(Clone)]
pub struct RunThreadCall {
    // the declared literal names plus any other string, which answers 400
    pub provider: String,
    pub thread_id: String,
    pub assistant_id: String,
    pub additional_instructions: Option<String>,
    pub instructions: Option<String>,
    pub metadata: Option<JsonValue>,
    pub model: Option<String>,
    // a Python default of null: absent is None, an explicit null Some(None), a value Some(Some(_))
    pub stream: Option<Option<RunFlag>>,
    pub tools: Option<JsonValue>,
    pub client: Option<RunClient>,
    pub event_handler: Option<Arc<dyn RunEventHandler>>,
    // run_thread pops this from **kwargs, so the public signatures never declare it
    pub arun_thread: Option<RunFlag>,
    pub params: RunParams,
}
