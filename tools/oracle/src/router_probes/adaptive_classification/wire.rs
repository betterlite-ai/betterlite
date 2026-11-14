use betterlite_models::decisions::Json;
use betterlite_router::adaptive_classification::request_type::RequestType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    pub(super) check: Check,
    pub(super) prompts: Vec<String>,
    pub(super) requests: Vec<RequestInput>,
    pub(super) deployments: Vec<DeploymentInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Check {
    Classify,
    MinimumQuality,
    RouterConfigs,
    Preferences,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestInput {
    pub(super) prompt: String,
    pub(super) headers: Option<Json>,
    pub(super) metadata: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeploymentInput {
    pub(super) model_name: String,
    pub(super) adaptive_router_config: Option<RouterConfigInput>,
    pub(super) adaptive_router_preferences: Option<PreferencesInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RouterConfigInput {
    pub(super) available_models: Vec<String>,
    pub(super) weights: Option<WeightsInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WeightsInput {
    pub(super) quality: Option<String>,
    pub(super) cost: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreferencesInput {
    pub(super) quality_tier: i64,
    pub(super) strengths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fields {
    pub(super) types: Option<Vec<RequestType>>,
    pub(super) tiers: Option<Vec<Option<i64>>>,
    pub(super) routers: Option<Vec<RouterOutput>>,
    pub(super) preferences: Option<Vec<PreferenceOutput>>,
    pub(super) invalid: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RouterOutput {
    pub(super) model_name: String,
    pub(super) available_models: Vec<String>,
    pub(super) quality: String,
    pub(super) cost: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PreferenceOutput {
    pub(super) model_name: String,
    pub(super) quality_tier: i64,
    pub(super) strengths: Vec<RequestType>,
}
