use std::collections::BTreeMap;

use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_models::decisions::{Extra, Json};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

mod call;
mod hooks;
mod source;
mod tests;
mod view;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Streaming,
    AsyncStreaming,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    cases: Vec<StreamCase>,
}

// azure.py streaming() / async_streaming()'s arguments as the oracle cases spell them
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamCase {
    pub(crate) model: String,
    pub(crate) api_base: String,
    pub(crate) api_version: String,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) azure_ad_token: Option<String>,
    #[serde(default)]
    pub(crate) azure_ad_token_provider: Option<u64>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    pub(crate) max_retries: i64,
    #[serde(default)]
    pub(crate) dynamic_params: bool,
    #[serde(default)]
    pub(crate) data: Extra,
    #[serde(default)]
    pub(crate) litellm_params: LiteLlmParams,
    #[serde(default)]
    pub(crate) supplied: Option<ClientSpec>,
    #[serde(default)]
    pub(crate) get_error: Option<ErrorSpec>,
    #[serde(default)]
    pub(crate) send: SendSpec,
    #[serde(default)]
    pub(crate) exchange: ExchangeSpec,
}

// A python value that may be missing, explicitly None, or set
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Presence<T> {
    pub(crate) present: bool,
    #[serde(default)]
    pub(crate) value: Option<T>,
}

impl<T> Default for Presence<T> {
    fn default() -> Self {
        Self {
            present: false,
            value: None,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClientKindSpec {
    AzureSync,
    AzureAsync,
    OpenAiSync,
    OpenAiAsync,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientSpec {
    pub(crate) kind: ClientKindSpec,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
}

// The exception the scripted get_azure_openai_client raises
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorSpec {
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) status: Presence<u16>,
    #[serde(default)]
    pub(crate) message: Presence<String>,
    #[serde(default)]
    pub(crate) headers: Presence<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) body: Presence<Json>,
    #[serde(default)]
    pub(crate) response: Presence<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendSpec {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) events: Vec<String>,
    #[serde(default)]
    pub(crate) body: Option<String>,
}

impl Default for SendSpec {
    fn default() -> Self {
        Self {
            status: default_status(),
            headers: BTreeMap::new(),
            events: Vec::new(),
            body: None,
        }
    }
}

fn default_status() -> u16 {
    200
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExchangeSpec {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) assertion: String,
}

impl Default for ExchangeSpec {
    fn default() -> Self {
        Self {
            environment: BTreeMap::new(),
            status: default_status(),
            body: String::new(),
            assertion: String::new(),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        call::run(input)
    }
}
