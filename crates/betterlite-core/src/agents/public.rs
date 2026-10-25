use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMakePublicResponse {
    pub message: String,
    pub public_agent_groups: Vec<String>,
    pub updated_by: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MakeAgentsPublicRequest {
    pub agent_ids: Vec<String>,
}
