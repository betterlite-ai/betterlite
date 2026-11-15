use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::JsonObject;
use super::db::PolicyDBResponse;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyVersionCreateRequest {
    #[serde(default)]
    pub source_policy_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// ast-grep-ignore: no-long-names
pub struct PolicyVersionStatusUpdateRequest {
    pub version_status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyVersionListResponse {
    pub policy_name: String,
    #[serde(default)]
    pub versions: Vec<PolicyDBResponse>,
    #[serde(default)]
    pub total_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyVersionCompareResponse {
    pub version_a: PolicyDBResponse,
    pub version_b: PolicyDBResponse,
    #[serde(default)]
    pub field_diffs: BTreeMap<String, JsonObject>,
}
