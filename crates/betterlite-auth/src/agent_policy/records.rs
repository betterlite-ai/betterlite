use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentCaller {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentKeySummary {
    pub token: String,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub key_name: Option<String>,
}
