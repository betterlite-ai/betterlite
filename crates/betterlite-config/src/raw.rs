use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::providers::bedrock::request_policy::RawMetadataField;
use crate::types::{RouterFallback, RoutingStrategy};

#[derive(Debug, Deserialize, Default)]
pub(super) struct RawConfig {
    #[serde(default)]
    pub(super) model_list: Vec<RawDeployment>,
    #[serde(default)]
    pub(super) general_settings: RawGeneral,
    #[serde(default)]
    pub(super) router_settings: RawRouter,
    #[serde(default)]
    pub(super) litellm_settings: RawLiteLlm,
    #[serde(default)]
    pub(super) environment_variables: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RawDeployment {
    pub(super) model_name: String,
    #[serde(default)]
    pub(super) litellm_params: RawParams,
    #[serde(default)]
    pub(super) model_info: RawInfo,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RawParams {
    pub(super) model: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) timeout: Option<RawNumber>,
    pub(super) stream_timeout: Option<RawNumber>,
    pub(super) max_retries: Option<RawNumber>,
    pub(super) rpm: Option<RawNumber>,
    pub(super) tpm: Option<RawNumber>,
    pub(super) weight: Option<RawNumber>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RawInfo {
    pub(super) id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RawGeneral {
    pub(super) master_key: Option<String>,
    pub(super) encryption_algorithm: Option<RawAlgorithm>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawAlgorithm {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RawRouter {
    pub(super) routing_strategy: Option<RoutingStrategy>,
    pub(super) num_retries: Option<RawNumber>,
    pub(super) timeout: Option<RawNumber>,
    pub(super) allowed_fails: Option<RawNumber>,
    pub(super) cooldown_time: Option<RawNumber>,
    #[serde(default)]
    pub(super) fallbacks: Vec<RouterFallback>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RawLiteLlm {
    pub(super) num_retries: Option<RawNumber>,
    pub(super) request_timeout: Option<RawNumber>,
    pub(super) drop_params: Option<bool>,
    #[serde(default)]
    pub(super) aiohttp_trust_env: bool,
    #[serde(default)]
    pub(super) disable_aiohttp_trust_env: bool,
    #[serde(rename = "bedrock_request_metadata_fields")]
    pub(super) bedrock_metadata_fields: Option<Vec<RawMetadataField>>,
    pub(super) modify_params: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawNumber {
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Text(String),
}
