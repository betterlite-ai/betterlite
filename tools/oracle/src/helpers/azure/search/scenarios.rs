use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::SearchQuery;
use serde::Deserialize;

pub(crate) type Params = Object;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestScenario {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) query: SearchQuery,
    #[serde(default)]
    pub(crate) optional_params: Params,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    #[serde(default)]
    pub(crate) scenarios: Vec<RequestScenario>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MinterSpec {
    pub(crate) token: String,
    #[serde(default)]
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderScenario {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    pub(crate) minter: MinterSpec,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderInput {
    #[serde(default)]
    pub(crate) scenarios: Vec<HeaderScenario>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyScenario {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) status: Option<u16>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) optional_params: Option<Params>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    #[serde(default)]
    pub(crate) scenarios: Vec<ReplyScenario>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapScenario {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) query: SearchQuery,
    #[serde(default)]
    pub(crate) optional_params: Params,
    #[serde(default)]
    pub(crate) status: Option<u16>,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaxResultsInput {
    #[serde(default)]
    pub(crate) scenarios: Vec<CapScenario>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlScenario {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsInput {
    #[serde(default)]
    pub(crate) urls: Vec<UrlScenario>,
    #[serde(default)]
    pub(crate) requests: Vec<RequestScenario>,
}
