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
#[serde(from = "PermissionGetResponseInput")]
pub struct PermissionGetResponse {
    pub team_id: String,
    pub team_member_permissions: Option<Vec<String>>,
    pub all_available_permissions: Vec<String>,
}

#[derive(Deserialize)]
struct PermissionGetResponseInput {
    team_id: String,
    #[serde(default, deserialize_with = "serde_with::rust::double_option::deserialize")]
    team_member_permissions: Option<Option<Vec<String>>>,
    all_available_permissions: Vec<String>,
}

impl From<PermissionGetResponseInput> for PermissionGetResponse {
    fn from(input: PermissionGetResponseInput) -> Self {
        Self {
            team_id: input.team_id,
            team_member_permissions: input.team_member_permissions.unwrap_or(Some(Vec::new())),
            all_available_permissions: input.all_available_permissions,
        }
    }
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
