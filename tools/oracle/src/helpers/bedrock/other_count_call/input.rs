use std::collections::BTreeMap;

use betterlite_bedrock::count_call::counter::CountTokensDeployment;
use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    CountTokenHttpRequest(Calls),
    NativeCountSelection(Calls),
    CountApiFailureWarning(Calls),
    CountCredentials(Calls),
    CountTimeout(Calls),
    CounterFactory(Calls),
    CountResultOrNone(Calls),
    CountDebugEvents(Calls),
}

impl Input {
    pub(crate) const fn calls(&self) -> &Calls {
        match self {
            Self::CountTokenHttpRequest(calls)
            | Self::NativeCountSelection(calls)
            | Self::CountApiFailureWarning(calls)
            | Self::CountCredentials(calls)
            | Self::CountTimeout(calls)
            | Self::CounterFactory(calls)
            | Self::CountResultOrNone(calls)
            | Self::CountDebugEvents(calls) => calls,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Calls {
    #[serde(default)]
    pub(crate) calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Option<Json>,
    // LiteLLM never reads this argument; the helper accepts it to prove the counter ignores it
    #[serde(default, rename = "contents")]
    pub(crate) _contents: Option<Json>,
    #[serde(default)]
    pub(crate) tools: Option<Json>,
    #[serde(default)]
    pub(crate) system: Option<Json>,
    #[serde(default)]
    pub(crate) request_model: String,
    #[serde(default)]
    pub(crate) deployment: Option<CountTokensDeployment>,
    #[serde(default)]
    pub(crate) environment: Environment,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) handler: Handler,
    #[serde(default)]
    pub(crate) reply: Reply,
    pub(crate) now: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Environment {
    #[serde(default)]
    pub(crate) region_name: Option<String>,
    #[serde(default)]
    pub(crate) region: Option<String>,
    #[serde(default)]
    pub(crate) session_region: Option<String>,
    #[serde(default)]
    pub(crate) runtime_endpoint: Option<String>,
    #[serde(default)]
    pub(crate) bearer_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Handler {
    #[default]
    Real,
    None,
    EmptyObject,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default = "default_body")]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) failure: bool,
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            status: default_status(),
            body: default_body(),
            headers: BTreeMap::new(),
            failure: false,
        }
    }
}

const fn default_status() -> u16 {
    200
}

fn default_body() -> String {
    "{}".to_owned()
}
