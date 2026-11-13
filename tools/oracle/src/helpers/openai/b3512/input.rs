use std::collections::BTreeMap;

use betterlite_models::decisions::Extra;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Runtime(RuntimeStep),
    Config,
    Sign(SignStep),
    Transform(TransformStep),
}

#[derive(Debug, Deserialize)]
pub(crate) struct RuntimeStep {
    pub(crate) api_base: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SignStep {
    pub(crate) api_base: String,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) litellm_params: Extra,
    pub(crate) model: String,
    pub(crate) state: String,
    pub(crate) question: String,
    pub(crate) instructions: String,
    pub(crate) now: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TransformStep {
    pub(crate) body: String,
    pub(crate) model: String,
    pub(crate) state: String,
    pub(crate) question: String,
    pub(crate) instructions: String,
}
