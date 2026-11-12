use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) non_default_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) existing_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) reply: Reply,
    #[serde(default)]
    pub(crate) failure: Option<Failure>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) organization: Option<String>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) timeout: Option<i64>,
    #[serde(default)]
    pub(crate) is_async: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            status: 200,
            headers: BTreeMap::new(),
            body: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Timeout,
    Failed,
}
