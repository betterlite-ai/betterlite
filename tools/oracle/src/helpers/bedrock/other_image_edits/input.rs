use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use betterlite_openai::api::image_contracts::records::ImageParams;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) model_cost: ParamMap,
    pub(crate) probes: Vec<Probe>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Prepare,
    Result,
    Error,
    ModelError,
    Selection,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) images: Vec<String>,
    #[serde(default)]
    pub(crate) optional_params: ImageParams,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) configured_bearer_token: Option<String>,
    #[serde(default)]
    pub(crate) environment: Environment,
    #[serde(default)]
    pub(crate) credentials: Credentials,
    #[serde(default)]
    pub(crate) reply: Reply,
    #[serde(default)]
    pub(crate) supplied: Supplied,
    #[serde(default)]
    pub(crate) aimage_edit: bool,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Environment {
    pub(crate) region_name: Option<String>,
    pub(crate) region: Option<String>,
    pub(crate) session_region: Option<String>,
    pub(crate) runtime_endpoint: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Credentials {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    pub(crate) session_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
    pub(crate) failure: Option<ReplyFailure>,
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            status: 200,
            headers: BTreeMap::new(),
            body: "{\"images\": [\"eA==\"]}".to_owned(),
            failure: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplyFailure {
    Timeout,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Supplied {
    #[default]
    None,
    Sync,
    Async,
}
