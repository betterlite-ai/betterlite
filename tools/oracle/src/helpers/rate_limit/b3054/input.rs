use std::collections::BTreeMap;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_db::identity_rows::Payload;
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
    pub(super) memory: BTreeMap<String, i64>,
    #[serde(default)]
    pub(super) redis_values: BTreeMap<String, i64>,
    #[serde(default)]
    pub(super) agents: Vec<AgentResponse>,
    #[serde(default)]
    pub(super) calls: Vec<Call>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum RedisMode {
    #[default]
    Absent,
    Healthy,
    ScriptFails,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Call {
    #[serde(default)]
    pub(super) agent_id: Option<String>,
    #[serde(default)]
    pub(super) key_metadata: Option<Payload>,
    pub(super) data: Json,
}
