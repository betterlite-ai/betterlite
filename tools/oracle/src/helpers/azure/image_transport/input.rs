use std::collections::BTreeMap;

use betterlite_azure::image_transport::json::JsonMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) api_base: String,
    pub(crate) api_version: String,
    #[serde(default)]
    pub(crate) deployment_name: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) data: JsonMap,
    #[serde(default)]
    pub(crate) is_async: bool,
    #[serde(default)]
    pub(crate) supplied_client: bool,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) replies: Vec<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) delay: u64,
}
