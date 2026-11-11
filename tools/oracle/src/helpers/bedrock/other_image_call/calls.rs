use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Supplied {
    #[default]
    None,
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Timeout,
    Failed,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Credentials {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    pub(crate) session_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Environment {
    pub(crate) region_name: Option<String>,
    pub(crate) region: Option<String>,
    pub(crate) session_region: Option<String>,
    pub(crate) runtime_endpoint: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
    pub(crate) failure: Option<Failure>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Probe {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) configured_bearer_token: Option<String>,
    pub(crate) environment: Environment,
    pub(crate) credentials: Credentials,
    pub(crate) now: u64,
    pub(crate) reply: Reply,
    pub(crate) supplied: Supplied,
    pub(crate) aimg_generation: bool,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) created: i64,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            model: "amazon.titan-image-generator-v2:0".to_owned(),
            prompt: "a cat".to_owned(),
            optional_params: Map::new(),
            extra_headers: None,
            api_base: None,
            api_key: None,
            configured_bearer_token: None,
            environment: Environment::default(),
            credentials: Credentials::default(),
            now: 1_700_000_000,
            reply: Reply::default(),
            supplied: Supplied::default(),
            aimg_generation: false,
            timeout_ms: None,
            created: 42,
        }
    }
}
