use std::collections::BTreeMap;

use betterlite_bedrock::aws_endpoints::provider::BedrockInvokeProvider;
use betterlite_bedrock::aws_endpoints::runtime::BedrockEndpointType;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    ModelArnRegion { models: Vec<Option<String>> },
    LlmRegionName { cases: Vec<LlmRegionCase> },
    NonLlmRegionName { cases: Vec<NonLlmRegionCase> },
    InvalidRegion { values: Vec<Option<String>> },
    ModelIdEncoding { model_ids: Vec<String> },
    BedrockModelId { cases: Vec<BedrockModelIdCase> },
    ProviderSelection { models: Vec<String> },
    DefaultEndpoint { cases: Vec<DefaultEndpointCase> },
    RuntimeEndpoint { cases: Vec<RuntimeEndpointCase> },
    StsEndpointRegion { endpoints: Vec<Option<String>> },
    StsKwargs { cases: Vec<StsKwargsCase> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LlmRegionCase {
    #[serde(default)]
    pub(super) optional_params: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) model_id: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) session: SessionFixture,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionFixture {
    #[serde(default)]
    pub(super) region_name: Option<String>,
    #[serde(default)]
    pub(super) error: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NonLlmRegionCase {
    #[serde(default)]
    pub(super) explicit: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BedrockModelIdCase {
    #[serde(default)]
    pub(super) optional_params: BTreeMap<String, Box<RawValue>>,
    #[serde(default)]
    pub(super) provider: Option<BedrockInvokeProvider>,
    pub(super) model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DefaultEndpointCase {
    #[serde(default)]
    pub(super) endpoint_type: Option<BedrockEndpointType>,
    pub(super) region: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeEndpointCase {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) aws_bedrock_runtime_endpoint: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) region: String,
    #[serde(default)]
    pub(super) endpoint_type: Option<BedrockEndpointType>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StsKwargsCase {
    #[serde(default)]
    pub(super) endpoint: Option<String>,
    #[serde(default)]
    pub(super) verify: Option<bool>,
    #[serde(default)]
    pub(super) configured_region: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}
