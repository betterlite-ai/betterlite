use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::registry::PolicySource;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyAttachmentCreateRequest {
    pub policy_name: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub teams: Option<Vec<String>>,
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    #[serde(default)]
    pub models: Option<Vec<String>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyAttachmentDBResponse {
    pub attachment_id: String,
    pub policy_name: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub teams: Vec<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_by: Option<String>,
    #[serde(default)]
    pub definition_location: PolicySource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyAttachmentListResponse {
    #[serde(default)]
    pub attachments: Vec<PolicyAttachmentDBResponse>,
    #[serde(default)]
    pub total_count: i64,
}
