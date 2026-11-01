use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::Timestamp;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessGroupTable {
    pub access_group_id: String,
    pub access_group_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub access_model_names: Vec<String>,
    #[serde(default)]
    pub access_mcp_server_ids: Vec<String>,
    #[serde(default)]
    pub access_agent_ids: Vec<String>,
    #[serde(default)]
    pub assigned_team_ids: Vec<String>,
    #[serde(default)]
    pub assigned_key_ids: Vec<String>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_by: Option<String>,
}
