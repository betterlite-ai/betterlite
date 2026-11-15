use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileSpec {
    pub(crate) content_base64: String,
    pub(crate) name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Params {
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) organization: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Globals {
    pub(crate) api_key: Option<String>,
    pub(crate) openai_key: Option<String>,
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
}

const fn default_status() -> u16 {
    200
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub(crate) model: String,
    pub(crate) prompt: Option<String>,
    pub(crate) image: Option<Vec<Option<FileSpec>>>,
    pub(crate) mask: Option<Vec<Option<FileSpec>>>,
    #[serde(default)]
    pub(crate) options: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) params: Params,
    #[serde(default)]
    pub(crate) globals: Globals,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) reply: Option<Reply>,
}
