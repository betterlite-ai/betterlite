use serde::{Deserialize, Serialize};

use super::values::KeyManagementRoutes;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionGetRequest {
    pub team_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionUpdateRequest {
    pub team_id: String,
    pub team_member_permissions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionGetResponse {
    pub team_id: String,
    #[serde(default = "default_permissions")]
    pub team_member_permissions: Option<Vec<String>>,
    pub all_available_permissions: Vec<String>,
}

const fn default_permissions() -> Option<Vec<String>> {
    Some(Vec::new())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkPermissionUpdateRequest {
    pub permissions: Vec<KeyManagementRoutes>,
    #[serde(default)]
    pub team_ids: Option<Vec<String>>,
    #[serde(default)]
    pub apply_to_all_teams: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkPermissionUpdateResponse {
    pub message: String,
    pub teams_updated: i64,
    #[serde(default)]
    pub permissions_appended: Option<Vec<String>>,
}
