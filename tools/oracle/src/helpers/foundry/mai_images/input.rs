use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) urls: Option<Vec<UrlCall>>,
    pub(super) models: Option<Vec<String>>,
    pub(super) params: Option<Vec<ParamsCall>>,
    pub(super) usages: Option<Vec<Option<Object>>>,
    pub(super) replies: Option<Vec<ReplyCall>>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OperationName {
    Generation,
    Edit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UrlCall {
    pub(super) operation: OperationName,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParamsCall {
    pub(super) model: String,
    pub(super) non_default_params: Object,
    pub(super) optional_params: Object,
    pub(super) drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyCall {
    pub(super) status_code: u16,
    pub(super) body: String,
    pub(super) request_data: Object,
    pub(super) optional_params: Object,
    pub(super) api_key: Option<String>,
}
