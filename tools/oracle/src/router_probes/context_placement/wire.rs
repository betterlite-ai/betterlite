use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigInput {
    #[serde(default)]
    pub enabled: bool,
    // a float travels as its decimal text, since the probe enums derive Eq
    #[serde(default)]
    pub buffer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeploymentInput {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub max_input_tokens: Option<i64>,
    #[serde(default)]
    pub max_output_tokens: Option<i64>,
    #[serde(default)]
    pub router_max_input_tokens: Option<u64>,
    #[serde(default)]
    pub router_max_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GroupInput {
    pub model: String,
    #[serde(default)]
    pub deployments: Vec<DeploymentInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    #[serde(default)]
    pub config: Option<ConfigInput>,
    pub tier: String,
    #[serde(default)]
    pub signals: Vec<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub pool_override: Option<Vec<String>>,
    pub pools: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub groups: Vec<GroupInput>,
    #[serde(default)]
    pub messages: Vec<String>,
    #[serde(default)]
    pub instructions: Option<Json>,
    #[serde(default)]
    pub system: Option<Json>,
    #[serde(default)]
    pub count: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WindowFact {
    pub window: Option<u64>,
    pub has_unknown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlacementOutput {
    pub tier: String,
    pub allowed_models: Vec<String>,
    pub holdable_models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AppliedOutput {
    pub tier: String,
    pub signals: Vec<String>,
    pub original_tier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Output {
    pub out_of_band: String,
    pub byte_bound: u64,
    pub facts: BTreeMap<String, WindowFact>,
    pub needed: Option<u64>,
    pub ceilings: BTreeMap<String, Option<u64>>,
    pub placement: Option<PlacementOutput>,
    pub applied: AppliedOutput,
}
