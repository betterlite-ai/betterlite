use chrono::NaiveDateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessGroupRecord {
    pub access_group_id: String,
    pub access_group_name: String,
    pub description: Option<String>,
    pub access_model_names: Option<Vec<String>>,
    pub access_mcp_server_ids: Option<Vec<String>>,
    pub access_agent_ids: Option<Vec<String>>,
    pub assigned_team_ids: Option<Vec<String>>,
    pub assigned_key_ids: Option<Vec<String>>,
    pub created_at: NaiveDateTime,
    pub created_by: Option<String>,
    pub updated_at: NaiveDateTime,
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccessGroup {
    pub access_group_name: String,
    pub description: Option<String>,
    pub access_model_names: Vec<String>,
    pub access_mcp_server_ids: Vec<String>,
    pub access_agent_ids: Vec<String>,
    pub assigned_team_ids: Vec<String>,
    pub assigned_key_ids: Vec<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
}

// Prisma's data map tells a nullable column it is absent, null, or a value; Option<Option<T>> cannot spell which
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldWrite<T> {
    Unchanged,
    Null,
    Set(T),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessGroupChanges {
    pub access_group_name: Option<String>,
    pub description: FieldWrite<String>,
    pub access_model_names: Option<Vec<String>>,
    pub access_mcp_server_ids: Option<Vec<String>>,
    pub access_agent_ids: Option<Vec<String>>,
    pub assigned_team_ids: Option<Vec<String>>,
    pub assigned_key_ids: Option<Vec<String>>,
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamRecord {
    pub team_id: String,
    pub team_alias: Option<String>,
    pub access_group_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRecord {
    pub token: String,
    pub access_group_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRecord {
    pub agent_id: String,
    pub access_group_ids: Option<Vec<String>>,
}
