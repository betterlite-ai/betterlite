use std::collections::BTreeMap;

use betterlite_openai::huggingface_embedding::request::Params;
use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Prepare(PrepareStep),
    TransformInput(TransformInputStep),
    PipelineTag(PipelineTagStep),
    OptionalParams(OptionalParamsStep),
    Response(ResponseStep),
    ValidateEnvironment(ValidateEnvironmentStep),
    Url(UrlStep),
}

#[derive(Debug, Deserialize)]
pub(crate) struct PrepareStep {
    pub(crate) model: String,
    pub(crate) input: Vec<String>,
    #[serde(default)]
    pub(crate) optional_params: Params,
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) pipeline_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TransformInputStep {
    pub(crate) model: String,
    pub(crate) input: Vec<String>,
    #[serde(default)]
    pub(crate) optional_params: Params,
    pub(crate) pipeline_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PipelineTagStep {
    pub(crate) input: Vec<String>,
    pub(crate) pipeline_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OptionalParamsStep {
    pub(crate) data: Object,
    #[serde(default)]
    pub(crate) optional_params: Params,
    #[serde(default)]
    pub(crate) ordered_params: Vec<(String, Value)>,
}

impl OptionalParamsStep {
    // the oracle input is a sorted JSON object, so a case that needs the client's key order sends pairs
    pub(crate) fn params(&self) -> Params {
        let mut params = self.optional_params.clone();
        params.extend(self.ordered_params.iter().cloned());
        params
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseStep {
    pub(crate) model: String,
    pub(crate) input: Vec<String>,
    pub(crate) embeddings: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ValidateEnvironmentStep {
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlStep {
    pub(crate) model: String,
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) task: Option<String>,
}
