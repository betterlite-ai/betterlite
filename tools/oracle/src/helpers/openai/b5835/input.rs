use std::collections::{BTreeMap, BTreeSet};

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    ProviderInfoAndSupportedParams(ProviderStep),
    MapParams(MapStep),
    EmbeddingHeadersAndUrl(HeadersStep),
    EmbeddingRequestAndParams(RequestStep),
    ModelsUrlAndList(ModelsStep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) chat_models: BTreeSet<String>,
    #[serde(default)]
    pub(crate) text_models: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) chat_models: BTreeSet<String>,
    #[serde(default)]
    pub(crate) text_models: BTreeSet<String>,
    #[serde(default)]
    pub(crate) non_default_params: Object,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeadersStep {
    #[serde(default)]
    pub(crate) headers: Headers,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestStep {
    pub(crate) model: String,
    pub(crate) input: EmbeddingInput,
    #[serde(default)]
    pub(crate) non_default_params: Object,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelsStep {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) status: u16,
    pub(crate) body: String,
}
