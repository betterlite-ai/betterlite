use std::collections::BTreeMap;

use betterlite_config::sections::tier_definitions::TierPool as WireTierPool;
use betterlite_router::router_predicates::JsonMap;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

// one Router model_list row: the group it serves, its provider model and what it declares
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeploymentInput {
    pub group: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub allowed_openai_params: Vec<String>,
}

// one request through the hook: the tier it landed on, the pool override and the body it carries
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StepInput {
    pub tier: String,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    #[serde(default)]
    pub responses_call: bool,
    #[serde(default)]
    pub request_kwargs: JsonMap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    #[serde(default)]
    pub return_raw_model_name: Option<bool>,
    #[serde(default)]
    pub max_tokens_from_tier_model: Option<bool>,
    #[serde(default)]
    pub tiers: IndexMap<String, WireTierPool>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub deployment_pin: bool,
    #[serde(default)]
    pub deployments: Vec<DeploymentInput>,
    #[serde(default)]
    pub supported_params: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub steps: Vec<StepInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StepOutput {
    pub model: String,
    pub tier: String,
    pub pinned: bool,
    pub request_kwargs: JsonMap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Output {
    pub tier_pools: BTreeMap<String, Vec<String>>,
    pub steps: Vec<StepOutput>,
}
