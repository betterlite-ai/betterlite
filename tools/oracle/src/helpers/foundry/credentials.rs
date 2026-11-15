use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fixture;
mod steps;
mod tests;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    AgentHeaders(Vec<AgentStep>),
    AgentTokens(Vec<AgentStep>),
    AuthHeaders(Vec<AuthStep>),
    HasEntra(Vec<EntraStep>),
    HostBoundary(Vec<HostStep>),
    NativeResponses(Vec<NativeStep>),
    ReasoningGate(Vec<GateStep>),
    Settings(Vec<SettingsStep>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostStep {
    api_base: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthStep {
    api_key: Option<String>,
    api_key_env_var: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    header: HeaderName,
    params: Option<Object>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentStep {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    params: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntraStep {
    params: Option<Object>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeStep {
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    global_api_base: Option<String>,
    model: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsStep {
    api_base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    global_api_base: Option<String>,
    global_api_key: Option<String>,
    params: Option<Object>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateStep {
    model: String,
    reasoning_effort: Option<Json>,
}

#[derive(Clone, Copy, Default, Deserialize)]
pub(crate) enum HeaderName {
    #[default]
    #[serde(rename = "Authorization")]
    Authorization,
    #[serde(rename = "api-key")]
    ApiKey,
    #[serde(rename = "Api-Key")]
    ApiKeyTitleCase,
    #[serde(rename = "Ocp-Apim-Subscription-Key")]
    OcpApimSubscriptionKey,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input)
    }
}
