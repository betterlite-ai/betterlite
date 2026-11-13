use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Url(UrlStep),
    SupportedParams(SupportedParamsStep),
    Params(ParamsStep),
    ValidateEnvironment(ValidateEnvironmentStep),
    Request(RequestStep),
    Response(ResponseStep),
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlStep {
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SupportedParamsStep {}

#[derive(Debug, Deserialize)]
pub(crate) struct ParamsStep {
    #[serde(default)]
    pub(crate) non_default_params: Option<BTreeMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ValidateEnvironmentStep {
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RequestStep {
    pub(crate) params: BTreeMap<String, Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseStep {
    pub(crate) model: String,
    pub(crate) status: u16,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) request_data: BTreeMap<String, Json>,
    pub(crate) token_counts: Option<Vec<Option<u64>>>,
    pub(crate) id: String,
}
