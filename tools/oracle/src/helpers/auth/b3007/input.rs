use std::collections::BTreeMap;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) ttl_env: Option<String>,
    #[serde(default)]
    pub(super) redis: RedisMode,
    #[serde(default)]
    pub(super) memory: BTreeMap<String, serde_json::Number>,
    #[serde(default)]
    pub(super) redis_values: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) agents: Vec<AgentResponse>,
    #[serde(default)]
    pub(super) pre_calls: Vec<PreCall>,
    #[serde(default)]
    pub(super) charges: Vec<Charge>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum RedisMode {
    #[default]
    Absent,
    Healthy,
    ReadFails,
    ScriptFails,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreCall {
    #[serde(default)]
    pub(super) agent_id: Option<String>,
    pub(super) data: Json,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Charge {
    #[serde(default)]
    pub(super) litellm_params: Option<Json>,
    #[serde(default)]
    pub(super) response_cost: Option<serde_json::Number>,
}
