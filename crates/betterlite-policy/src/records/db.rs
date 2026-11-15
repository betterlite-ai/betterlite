use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::registry::PolicySource;

use super::JsonObject;

const fn default_version_number() -> i32 {
    1
}

fn default_version_status() -> String {
    "production".to_owned()
}

const fn default_is_latest() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyDBResponse {
    pub policy_id: String,
    pub policy_name: String,
    #[serde(default = "default_version_number")]
    pub version_number: i32,
    #[serde(default = "default_version_status")]
    pub version_status: String,
    #[serde(default)]
    pub parent_version_id: Option<String>,
    #[serde(default = "default_is_latest")]
    pub is_latest: bool,
    #[serde(default)]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub production_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub inherit: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub guardrails_add: Vec<String>,
    #[serde(default)]
    pub guardrails_remove: Vec<String>,
    #[serde(default)]
    pub condition: Option<JsonObject>,
    #[serde(default)]
    pub pipeline: Option<JsonObject>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyListDBResponse {
    #[serde(default)]
    pub policies: Vec<PolicyDBResponse>,
    #[serde(default)]
    pub total_count: i64,
}
