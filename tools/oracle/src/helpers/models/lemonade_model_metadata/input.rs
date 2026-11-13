use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Origin {
        queries: Vec<OriginQuery>,
    },
    PositiveInt {
        values: Vec<Json>,
        #[serde(default)]
        model_infos: Vec<IndexMap<String, Json>>,
    },
    ProviderEntry {
        model_infos: Vec<IndexMap<String, Json>>,
    },
    ModelInfo {
        queries: Vec<ModelInfoQuery>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginQuery {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) global_keys: BTreeMap<String, String>,
}

const fn default_timeout_seconds() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

// LiteLLM builds its module-level client with litellm.request_timeout, whose package default is this
const DEFAULT_TIMEOUT_SECONDS: u64 = 6000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelInfoQuery {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) global_keys: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) reply: Option<Reply>,
    #[serde(default = "default_timeout_seconds")]
    pub(crate) timeout_seconds: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: Option<Json>,
    #[serde(default)]
    pub(crate) raw: Option<String>,
}
