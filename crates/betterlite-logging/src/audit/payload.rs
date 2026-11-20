use serde::{Deserialize, Serialize};

// LiteLLM's StandardAuditLogPayload TypedDict, the contract every audit_log_callbacks logger receives
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardAuditLogPayload {
    pub id: String,
    pub updated_at: String,
    pub changed_by: String,
    #[serde(rename = "changed_by_api_key")]
    pub hashed_changed_by_api_key: String,
    pub action: String,
    pub table_name: String,
    pub object_id: String,
    pub before_value: Option<String>,
    pub updated_values: Option<String>,
}
