use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) enum Input {
    #[serde(rename = "mantle_bearer_precedence")]
    BearerPrecedence { cases: Vec<BearerCase> },
    #[serde(rename = "mantle_region_prefix")]
    RegionPrefix { models: Vec<String> },
    #[serde(rename = "mantle_region_resolution")]
    RegionResolution { cases: Vec<RegionCase> },
    #[serde(rename = "mantle_model_capabilities")]
    ModelCapabilities {
        model_cost: ParamMap,
        models: Vec<Option<String>>,
    },
    #[serde(rename = "mantle_chat_endpoint_inputs")]
    ChatEndpoint {
        model_cost: ParamMap,
        cases: Vec<ChatCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BearerCase {
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionCase {
    #[serde(default)]
    pub(super) aws_region_name: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatCase {
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) aws_region_name: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}
