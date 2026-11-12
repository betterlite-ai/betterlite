use std::collections::BTreeMap;

use betterlite_bedrock::count_call::counter::CountTokensDeployment;
use betterlite_models::decisions::Json;
use serde::Deserialize;

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
    pub(crate) messages: Option<Json>,
    // LiteLLM never reads this argument; the helper accepts it to prove the counter ignores it
    #[serde(rename = "contents")]
    pub(crate) _contents: Option<Json>,
    pub(crate) tools: Option<Json>,
    pub(crate) system: Option<Json>,
    #[serde(default)]
    pub(crate) request_model: String,
    pub(crate) deployment: Option<CountTokensDeployment>,
    #[serde(default)]
    pub(crate) environment: Environment,
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
    pub(crate) region_name: Option<String>,
    pub(crate) region: Option<String>,
    pub(crate) session_region: Option<String>,
    pub(crate) runtime_endpoint: Option<String>,
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: Option<u16>,
    pub(crate) body: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) failure: bool,
}
