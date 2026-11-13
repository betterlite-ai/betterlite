mod calls;
mod operations;
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub(crate) body: Option<String>,
    pub(crate) status: Option<u16>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) now: Option<i64>,
    pub(crate) event: Option<serde_json::Value>,
    pub(crate) chunks: Option<Vec<String>>,
    pub(crate) model: Option<String>,
    pub(crate) stream: Option<bool>,
    pub(crate) provider: Option<String>,
    pub(crate) native: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) litellm_api_key: Option<String>,
    pub(crate) litellm_openai_key: Option<String>,
    pub(crate) env_api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) litellm_api_base: Option<String>,
    pub(crate) env_base_url: Option<String>,
    pub(crate) env_api_base: Option<String>,
    pub(crate) identity: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probes {
    pub(crate) probes: Vec<Probe>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    CreatedAtConversion(Probes),
    LaxResponseValidation(Probes),
    MissingErrorCode(Probes),
    UnknownGenericEvent(Probes),
    CompletedIncompleteFailed(Probes),
    LastTerminalWins(Probes),
    NoTerminalEvent(Probes),
    NonNativeStreamingModel(Probes),
    KnownEventVariants(Probes),
    ResponsesAuthHeaders(Probes),
    ResponsesBaseUrl(Probes),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        calls::run(input)
    }
}
