use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) api_base: String,
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) caller_headers: IndexMap<String, String>,
    pub(crate) api_key: Option<String>,
    pub(crate) azure_ad_token: Option<String>,
    pub(crate) upstream_status: Option<u16>,
    pub(crate) upstream_headers: IndexMap<String, String>,
    pub(crate) upstream_body: Option<String>,
    pub(crate) upstream_lines: Vec<String>,
}
