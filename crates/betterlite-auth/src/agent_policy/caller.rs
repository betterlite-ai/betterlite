use serde::{Deserialize, Serialize};

pub const CALLER_USER_ID_HEADER: &str = "x-litellm-user-id";
pub const CALLER_TEAM_ID_HEADER: &str = "x-litellm-team-id";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentCaller {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
}
