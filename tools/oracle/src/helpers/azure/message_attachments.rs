use std::collections::BTreeMap;

use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::threads::records::ThreadMessageData;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fixture;
mod reports;
mod steps;
mod tests;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuppliedStep {
    kind: ClientKind,
}

// One facade call: the method arguments LiteLLM reads, the litellm_params it forwards, and the SDK reply to serve.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    #[serde(default)]
    params: LiteLlmParams,
    #[serde(default)]
    api_key: Option<SecretString>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_version: Option<String>,
    #[serde(default)]
    azure_ad_token: Option<SecretString>,
    #[serde(default)]
    timeout: Option<f64>,
    #[serde(default)]
    max_retries: Option<i64>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    flag: Option<Value>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    data: Option<ThreadMessageData>,
    #[serde(default)]
    supplied: Option<SuppliedStep>,
    #[serde(default)]
    reply: Option<Reply>,
}

// One legacy mapping call: the dict LiteLLM iterates, the dict it fills in, and the global drop_params flag.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapStep {
    non_default: Object,
    #[serde(default)]
    optional: Object,
    #[serde(default)]
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    AppendMessage(Vec<Step>),
    ListMessages(Vec<Step>),
    MapCreateMessageParams(Vec<MapStep>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input)
    }
}
