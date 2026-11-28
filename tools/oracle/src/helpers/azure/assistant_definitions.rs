use std::collections::BTreeMap;

use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::records::AssistantCreate;
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
    data: Option<AssistantCreate>,
    #[serde(default)]
    assistant_id: Option<String>,
    #[serde(default)]
    kind: ClientKind,
    #[serde(default)]
    supplied: Option<SuppliedStep>,
    #[serde(default)]
    reply: Option<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    ListAssistants(Vec<Step>),
    CreateAssistant(Vec<Step>),
    DeleteAssistant(Vec<Step>),
    ClientArguments(Vec<Step>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input)
    }
}
