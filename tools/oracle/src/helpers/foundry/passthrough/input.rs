use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Object as ParamsObject;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) roots: Vec<RootStep>,
    #[serde(default)]
    pub(super) prefixes: Vec<PrefixStep>,
    #[serde(default)]
    pub(super) queries: Vec<QueryStep>,
    #[serde(default)]
    pub(super) bases: Vec<BaseStep>,
    #[serde(default)]
    pub(super) validations: Vec<ValidationStep>,
    #[serde(default)]
    pub(super) nonstreams: Vec<NonStreamStep>,
    #[serde(default)]
    pub(super) streams: Vec<CollectedStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RootStep {
    pub(super) api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrefixStep {
    pub(super) root: String,
    pub(super) native_endpoint: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryStep {
    pub(super) api_base: String,
    pub(super) deployment_api_version: Option<String>,
    pub(super) request_query_params: Option<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BaseStep {
    pub(super) api_base: Option<String>,
    pub(super) global_api_base: Option<String>,
    pub(super) environment_api_base: Option<String>,
    pub(super) model: String,
    pub(super) endpoint: String,
    pub(super) litellm_params: Option<Object>,
    pub(super) request_query_params: Option<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ValidationStep {
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) params: Option<ParamsObject>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NonStreamStep {
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) model: String,
    pub(super) endpoint: String,
    pub(super) request_url: Option<String>,
    pub(super) ocr: Option<OcrStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OcrStep {
    pub(super) response: Option<String>,
    #[serde(default)]
    pub(super) malformed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CollectedStep {
    pub(super) model: String,
    pub(super) endpoint: String,
    #[serde(default)]
    pub(super) chunks: Vec<String>,
    #[serde(default)]
    pub(super) request_data: Object,
}
