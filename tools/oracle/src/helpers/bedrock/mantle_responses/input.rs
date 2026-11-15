use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) enum Input {
    #[serde(rename = "mantle_responses_url")]
    Url { cases: Vec<UrlCase> },
    #[serde(rename = "mantle_response_options")]
    ResponseOptions { cases: Vec<MapCase> },
    #[serde(rename = "mantle_model_name")]
    ModelName { cases: Vec<MapCase> },
    #[serde(rename = "unsupported_params_400")]
    UnsupportedParams400 { cases: Vec<MapCase> },
    #[serde(rename = "mantle_response_auth")]
    ResponseAuth { cases: Vec<AuthCase> },
    #[serde(rename = "unsupported_params_warn_drop")]
    UnsupportedParamsWarnDrop { cases: Vec<MapCase> },
    #[serde(rename = "openai_project_header")]
    ProjectHeader { cases: Vec<AuthCase> },
    #[serde(rename = "mantle_tool_filter")]
    ToolFilter { cases: Vec<MapCase> },
    #[serde(rename = "mantle_input_hoisting")]
    Hoisting { cases: Vec<HoistCase> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCase {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) aws_region_name: Option<String>,
    #[serde(default)]
    pub(super) use_openai_path: Option<bool>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCase {
    pub(super) model: String,
    #[serde(default)]
    pub(super) params: serde_json::Map<String, Value>,
    #[serde(default)]
    pub(super) drop_params: bool,
    #[serde(default)]
    pub(super) use_openai_path: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthCase {
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) project_id: Option<String>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) error: Option<ErrorProbe>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorProbe {
    pub(super) status: u16,
    pub(super) message: String,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HoistCase {
    pub(super) model: String,
    pub(super) input: Value,
    #[serde(default)]
    pub(super) params: Object,
}
